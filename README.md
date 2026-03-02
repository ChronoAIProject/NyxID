# NyxID

**NyxID** is a self-hosted authentication and single sign-on (SSO) platform. Named after Nyx, the Greek goddess and protector of darkness, NyxID guards the boundary between your users and your services.

It provides a complete identity layer: user registration, session management, OpenID Connect, social login, multi-factor authentication, API key management, and a reverse proxy that injects credentials into downstream service requests.

---

## Table of Contents

- [Features](#features)
- [Architecture Overview](#architecture-overview)
- [Prerequisites](#prerequisites)
- [Quick Start](#quick-start)
- [API Documentation](#api-documentation)
- [Environment Variables](#environment-variables)
- [Database Schema](#database-schema)
- [Security](#security)
- [MCP Integration](#mcp-integration)
- [Development Guide](#development-guide)
- [Project Structure](#project-structure)
- [License](#license)

---

## Features

### Authentication and Session Management
- Email/password registration with Argon2id hashing (OWASP-recommended parameters)
- Session-based authentication with HttpOnly, SameSite cookies
- JWT access and refresh tokens signed with RS256 (4096-bit RSA keys)
- Token rotation on refresh for replay attack prevention

### OpenID Connect Provider
- Full Authorization Code flow with mandatory PKCE (S256)
- ID token issuance following OpenID Connect Core
- UserInfo endpoint
- Support for both confidential and public clients

### Social Login
- Google and GitHub OAuth 2.0 integration
- Automatic account linking by verified email
- Session creation on successful social login (same cookies as email/password login)

### Multi-Factor Authentication (MFA)
- TOTP-based second factor (compatible with Google Authenticator, Authy, 1Password)
- QR code provisioning
- Recovery codes for account recovery
- MFA secrets encrypted at rest

### API Key Management
- Create, list, rotate, and revoke scoped API keys
- Key prefix display for identification (full key shown only at creation)
- SHA-256 hashed storage (plaintext never persisted)
- Optional expiration dates
- Last-used tracking

### Downstream Service Proxy
- Reverse proxy to internal or external services
- Three service categories: **provider** (OIDC/SSO), **connection** (per-user credentials), **internal** (master credential)
- Automatic credential injection: header, bearer token, query parameter, or basic auth
- Developer-friendly slug-based proxy URLs (`/api/v1/proxy/s/{slug}/{path}`) alongside UUID-based URLs
- Service discovery endpoint (`GET /api/v1/proxy/services`) for listing available services with proxy URLs and connection status
- Connection enforcement: users must connect before proxying; per-user credentials for connection services, master credentials for internal services
- SSRF protection (blocks private IPs, metadata endpoints, localhost)
- Path traversal prevention (rejects `..` and `//` in proxy paths)
- Header allowlist to prevent leaking sensitive request headers

### Service Connection Management
- Register downstream services with encrypted credentials (AES-256-GCM)
- Per-user encrypted credential storage for connection services
- Credential update without disconnect/reconnect
- Secure credential cleanup on disconnect and service deactivation
- Single source of truth for mapping users to downstream APIs

### Administration
- Full admin user management: list, view, edit, delete users
- Role management: promote/demote admin privileges (with self-protection)
- Account control: enable/disable users with automatic session and API key revocation
- Force password reset with session revocation
- Manual email verification
- Per-user session listing and bulk session revocation
- Cascade user deletion across 8 related collections (audit logs preserved)
- Audit log with action, resource, IP, and user-agent tracking (filterable by user)
- OAuth client management (create, list, deactivate)

### Roles and Groups (RBAC)
- Role definitions with permission string tags (e.g., `users:read`, `users:write`)
- Realm-level and client-scoped roles
- System roles (`admin`, `user`) seeded at startup and protected from deletion
- Default roles auto-assigned to new users
- Groups with role inheritance: all group members inherit the group's roles
- Hierarchical groups with optional parent-child relationships
- Direct role assignment to users and indirect assignment via group membership
- Effective permissions computed from direct roles + group-inherited roles
- New scopes (`roles`, `groups`) control whether RBAC claims appear in tokens
- Admin CRUD for roles, groups, role assignment, and group membership

### Token Introspection and Revocation
- Token introspection endpoint (RFC 7662): validates access and refresh tokens, returns active status with claims
- Token revocation endpoint (RFC 7009): revokes refresh tokens; access tokens expire naturally
- Both endpoints require client authentication (`client_id` + `client_secret`)
- Introspection response includes RBAC claims (`roles`, `groups`, `permissions`) when present

### User Consent Management
- Users can view all OAuth consents granted to third-party applications
- Per-client consent revocation without disconnecting from the application
- Consent records track granted scopes, grant time, and optional expiration

### Service Accounts
- Non-human (machine-to-machine) identities for programmatic access
- OAuth2 Client Credentials Grant authentication (`POST /oauth/token` with `grant_type=client_credentials`)
- Admin CRUD for service accounts with paginated listing and search
- Client secret generation with SHA-256 hashed storage (plaintext shown once at creation)
- Client secret rotation with automatic token revocation
- Scope-based access control (requested scopes must be a subset of allowed scopes)
- Token revocation support with per-token tracking via JTI
- Service accounts can access proxy, LLM gateway, connections, and provider endpoints
- RBAC role assignment for service accounts (direct roles, no group membership)
- Blocked from human-only endpoints (auth, users, sessions, admin, MFA)
- Configurable token TTL (default: 1 hour)
- Full audit logging for all service account operations

### Credential Broker
- Admin-managed provider registry (OpenAI, Anthropic, Google AI, Mistral, Cohere, etc.)
- Users connect by entering API keys or completing OAuth2 flows
- All credentials encrypted at rest (AES-256-GCM) with secure memory cleanup (zeroize)
- Credential delegation: downstream services declare provider requirements, proxy injects user tokens automatically
- Lazy OAuth token refresh with 5-minute buffer before expiry
- Token lifecycle tracking: active, expired, revoked, refresh_failed

### LLM Gateway
- Unified LLM access through NyxID: proxy requests to any supported LLM provider using stored credentials
- **Provider-specific endpoint:** `ANY /api/v1/llm/{provider_slug}/v1/{*path}` -- passthrough proxy to a specific provider's API
- **OpenAI-compatible gateway:** `ANY /api/v1/llm/gateway/v1/{*path}` -- routes requests by `model` field and translates between API formats
- **Status endpoint:** `GET /api/v1/llm/status` -- per-user provider readiness with proxy URLs
- Auto-seeded downstream services for 6 LLM providers at startup (no manual configuration required)
- Model-to-provider routing based on model name prefix (e.g., `gpt-*` to OpenAI, `claude-*` to Anthropic)
- Automatic Anthropic format translation: send OpenAI-format requests to Claude models through the gateway
- Google AI routed through its OpenAI-compatible endpoint automatically
- Supported providers: OpenAI, OpenAI Codex (OAuth), Anthropic, Google AI, Mistral, Cohere

### Identity Propagation
- Forward authenticated user identity to downstream services during proxy requests
- Four modes: `none`, `headers`, `jwt`, `both`
- Header mode: `X-NyxID-User-Id`, `X-NyxID-User-Email`, `X-NyxID-User-Name`
- JWT mode: Short-lived RS256-signed identity assertion (60-second TTL) via `X-NyxID-Identity-Token`
- Per-service configuration of which claims to include
- CRLF injection prevention on all header values

### Delegated Access
- Downstream services can make NyxID API calls (LLM gateway, proxy) on behalf of users
- Two complementary paths:
  - **MCP Injection:** NyxID automatically injects a short-lived delegation token (`X-NyxID-Delegation-Token`, 5-min TTL) when proxying MCP tool calls to services with `inject_delegation_token` enabled
  - **OAuth 2.0 Token Exchange (RFC 8693):** OIDC-linked services exchange a user's access token for a delegated token (5-minute TTL) via `POST /oauth/token` with `grant_type=urn:ietf:params:oauth:grant-type:token-exchange`
  - **Token Refresh:** Downstream services can renew delegation tokens during long-running/agentic workflows via `POST /api/v1/delegation/refresh`
- Delegated tokens are standard NyxID JWTs with `act.sub` (acting service) and `delegated: true` claims
- Scope enforcement: delegated tokens are restricted to proxy, LLM gateway, and delegation refresh endpoints; all other endpoints reject them via middleware
- Consent-gated: token exchange and every refresh validate that the user has active consent for the client; revoking consent immediately blocks further delegation
- Chained exchange prevention: delegated tokens cannot be exchanged for new delegated tokens
- Per-client `delegation_scopes` configuration controls which scopes can be requested
- Per-service `inject_delegation_token` and `delegation_token_scope` control MCP/proxy injection

### Security Hardening
- Rate limiting: per-IP sliding window with global token-bucket fallback
- Security headers: HSTS, CSP, X-Frame-Options (DENY), X-Content-Type-Options, Referrer-Policy, Permissions-Policy
- CORS restricted to a single configured frontend origin
- 1 MB global body size limit
- Input validation on all endpoints
- Structured error responses that never leak internal details
- Audit logging for all authentication events

---

## Architecture Overview

```
                         +------------------+
                         |   React 19 SPA   |
                         |  (Vite / Tailwind)|
                         +--------+---------+
                                  |
                            HTTPS | CORS
                                  |
                         +--------v---------+
                         |    Axum 0.8      |
                         |  (Rust Backend)  |
                         |                  |
                         |  +-- Middleware --+------> Rate Limiter
                         |  |  Security Hdr |------> CORS Layer
                         |  |  Auth Extract |------> JWT / Session
                         |  +---------------+
                         |                  |
                         |  +-- Handlers ---+
                         |  |  auth         |  POST /api/v1/auth/*
                         |  |  users        |  GET/PUT /api/v1/users/me
                         |  |  api_keys     |  CRUD /api/v1/api-keys
                         |  |  services     |  CRUD /api/v1/services
                         |  |  proxy        |  ANY  /api/v1/proxy/:id/*, /s/:slug/*
                         |  |  llm_gateway  |  ANY  /api/v1/llm/*
                         |  |  oauth        |  /oauth/authorize, /token, /userinfo
                         |  |  admin        |  /api/v1/admin/*
                         |  +---------------+
                         |                  |
                         |  +-- Services ---+
                         |  |  auth_service |  Registration, password verification
                         |  |  token_service|  JWT issuance, refresh rotation
                         |  |  oauth_service|  OIDC code exchange, client validation
                         |  |  key_service  |  API key CRUD, hashing
                         |  |  proxy_service|  Target resolution, request forwarding
                         |  |  llm_gateway  |  Model routing, format translation
                         |  |  mfa_service  |  TOTP generation, verification
                         |  |  audit_service|  Async audit log insertion
                         |  +---------------+
                         |                  |
                         +--------+---------+
                                  |
                            MongoDB Driver
                                  |
                         +--------v---------+
                         |  MongoDB 8.0     |
                         |  (19 collections)|
                         +------------------+
```

The backend follows a layered architecture:

1. **Middleware Layer** -- Rate limiting, security headers, authentication extraction
2. **Handler Layer** -- Request parsing, validation, response construction
3. **Service Layer** -- Business logic, orchestration
4. **Crypto Layer** -- Password hashing, JWT signing, AES encryption, token generation
5. **Model Layer** -- Document models mapping to MongoDB collections

---

## Prerequisites

| Tool       | Version   | Purpose                              |
|------------|-----------|--------------------------------------|
| Rust       | 1.85+     | Backend compiler                     |
| Node.js    | 20+       | Frontend build tooling               |
| MongoDB    | 8.0       | Primary database                     |
| Docker     | 24+       | Run MongoDB and Mailpit via Compose  |

---

## Quick Start

### 1. Clone and configure

```bash
git clone https://github.com/yourorg/NyxID.git
cd NyxID

cp .env.example .env
```

Edit `.env` and generate a real encryption key:

```bash
# Generate a 32-byte encryption key (required)
openssl rand -hex 32
```

Paste the output as the value of `ENCRYPTION_KEY` in `.env`.

### 2. Start infrastructure

```bash
docker compose up -d
```

This starts:
- **MongoDB 8.0** on port `27017` (database: `nyxid`)
- **Mailpit** SMTP on port `1025`, web UI on port `8025` (for dev email testing)

### 3. Initialize database

MongoDB collections are created automatically on first use. No manual migrations are required.

### 4. Start the backend

```bash
cargo run --manifest-path backend/Cargo.toml
```

The server starts on `http://localhost:3001`. RSA signing keys are auto-generated in development mode if the `keys/` directory does not exist.

### 5. Start the frontend

```bash
cd frontend
npm install
npm run dev
```

The frontend starts on `http://localhost:3000`.

### 6. Verify

```bash
curl http://localhost:3001/health
```

Expected response:

```json
{
  "status": "ok",
  "version": "0.1.0"
}
```

---

## API Documentation

All endpoints return JSON. Authenticated endpoints require either:
- A `Bearer <token>` header, or
- A valid `nyx_session` / `nyx_access_token` cookie

For the full API reference with request/response schemas and example curl commands, see **[docs/API.md](docs/API.md)**.

### Endpoint Summary

| Method | Path                                 | Auth     | Description                          |
|--------|--------------------------------------|----------|--------------------------------------|
| GET    | `/health`                            | None     | Health check                         |
| POST   | `/api/v1/auth/register`              | None     | Register a new user                  |
| POST   | `/api/v1/auth/login`                 | None     | Log in (returns tokens + cookies)    |
| POST   | `/api/v1/auth/logout`                | Required | Log out and revoke session           |
| POST   | `/api/v1/auth/refresh`               | Cookie   | Refresh access token                 |
| POST   | `/api/v1/auth/verify-email`          | None     | Verify email address with token      |
| POST   | `/api/v1/auth/forgot-password`       | None     | Request a password reset email       |
| POST   | `/api/v1/auth/reset-password`        | None     | Reset password with token            |
| GET    | `/api/v1/auth/social/{provider}`     | None     | Initiate social login (redirects to provider) |
| GET    | `/api/v1/auth/social/{provider}/callback` | None | Social login callback (exchanges code, creates session) |
| GET    | `/api/v1/users/me`                   | Required | Get current user profile             |
| PUT    | `/api/v1/users/me`                   | Required | Update current user profile          |
| GET    | `/api/v1/api-keys`                   | Required | List API keys                        |
| POST   | `/api/v1/api-keys`                   | Required | Create a new API key                 |
| DELETE | `/api/v1/api-keys/{key_id}`          | Required | Delete (deactivate) an API key       |
| POST   | `/api/v1/api-keys/{key_id}/rotate`   | Required | Rotate an API key                    |
| GET    | `/api/v1/services`                   | Required | List downstream services (`?category=` filter) |
| POST   | `/api/v1/services`                   | Admin    | Register a downstream service        |
| DELETE | `/api/v1/services/{service_id}`      | Admin    | Deactivate a downstream service      |
| GET    | `/api/v1/connections`                | Required | List user's service connections      |
| POST   | `/api/v1/connections/{service_id}`   | Required | Connect to a service (with credentials) |
| PUT    | `/api/v1/connections/{id}/credential`| Required | Update connection credential         |
| DELETE | `/api/v1/connections/{service_id}`   | Required | Disconnect from a service            |
| ANY    | `/api/v1/proxy/{service_id}/{*path}` | Required | Proxy request (requires connection)  |
| ANY    | `/api/v1/proxy/s/{slug}/{*path}`     | Required | Proxy request via service slug       |
| GET    | `/api/v1/proxy/services`             | Required | List proxyable services (paginated)  |
| GET    | `/oauth/authorize`                   | Required | OIDC authorization endpoint          |
| POST   | `/oauth/token`                       | None     | OIDC token endpoint (+ RFC 8693 token exchange) |
| GET    | `/oauth/userinfo`                    | Required | OIDC userinfo endpoint               |
| GET    | `/api/v1/admin/users`                | Admin    | List users (paginated, searchable)   |
| POST   | `/api/v1/admin/users`                | Admin    | Create a new user                    |
| GET    | `/api/v1/admin/users/{user_id}`      | Admin    | Get user details                     |
| PUT    | `/api/v1/admin/users/{user_id}`      | Admin    | Edit user profile                    |
| PATCH  | `/api/v1/admin/users/{user_id}/role` | Admin    | Toggle admin role                    |
| PATCH  | `/api/v1/admin/users/{user_id}/status`| Admin   | Enable/disable user                  |
| POST   | `/api/v1/admin/users/{user_id}/reset-password` | Admin | Force password reset        |
| DELETE | `/api/v1/admin/users/{user_id}`      | Admin    | Delete user (cascade)                |
| PATCH  | `/api/v1/admin/users/{user_id}/verify-email` | Admin | Manual email verification    |
| GET    | `/api/v1/admin/users/{user_id}/sessions` | Admin | List user sessions                |
| DELETE | `/api/v1/admin/users/{user_id}/sessions` | Admin | Revoke all user sessions         |
| GET    | `/api/v1/admin/audit-log`            | Admin    | Query audit log (paginated, filterable) |
| GET    | `/api/v1/admin/roles`                | Admin    | List all roles                        |
| POST   | `/api/v1/admin/roles`                | Admin    | Create a role                         |
| GET    | `/api/v1/admin/roles/{role_id}`      | Admin    | Get role details                      |
| PUT    | `/api/v1/admin/roles/{role_id}`      | Admin    | Update a role                         |
| DELETE | `/api/v1/admin/roles/{role_id}`      | Admin    | Delete a role                         |
| GET    | `/api/v1/admin/users/{user_id}/roles`| Admin    | Get user's direct and inherited roles |
| POST   | `/api/v1/admin/users/{user_id}/roles/{role_id}` | Admin | Assign role to user       |
| DELETE | `/api/v1/admin/users/{user_id}/roles/{role_id}` | Admin | Revoke role from user     |
| GET    | `/api/v1/admin/groups`               | Admin    | List all groups                       |
| POST   | `/api/v1/admin/groups`               | Admin    | Create a group                        |
| GET    | `/api/v1/admin/groups/{group_id}`    | Admin    | Get group details                     |
| PUT    | `/api/v1/admin/groups/{group_id}`    | Admin    | Update a group                        |
| DELETE | `/api/v1/admin/groups/{group_id}`    | Admin    | Delete a group                        |
| GET    | `/api/v1/admin/groups/{group_id}/members` | Admin | List group members                |
| POST   | `/api/v1/admin/groups/{group_id}/members/{user_id}` | Admin | Add member to group  |
| DELETE | `/api/v1/admin/groups/{group_id}/members/{user_id}` | Admin | Remove member from group |
| GET    | `/api/v1/admin/users/{user_id}/groups`| Admin   | Get user's groups                     |
| POST   | `/oauth/introspect`                  | None*    | Token introspection (RFC 7662)        |
| POST   | `/oauth/revoke`                      | None*    | Token revocation (RFC 7009)           |
| GET    | `/api/v1/users/me/consents`          | Required | List user's OAuth consents            |
| DELETE | `/api/v1/users/me/consents/{client_id}` | Required | Revoke consent for a client       |
| GET    | `/api/v1/providers`                  | Required | List provider configurations          |
| POST   | `/api/v1/providers`                  | Admin    | Register a provider                   |
| GET    | `/api/v1/providers/{id}`             | Required | Get a provider                        |
| PUT    | `/api/v1/providers/{id}`             | Admin    | Update a provider                     |
| DELETE | `/api/v1/providers/{id}`             | Admin    | Deactivate a provider                 |
| GET    | `/api/v1/providers/my-tokens`        | Required | List user's provider tokens           |
| POST   | `/api/v1/providers/{id}/connect/api-key` | Required | Connect via API key              |
| GET    | `/api/v1/providers/{id}/connect/oauth` | Required | Start OAuth connection flow         |
| GET    | `/api/v1/providers/callback`         | Required | Generic OAuth callback                |
| DELETE | `/api/v1/providers/{id}/disconnect`  | Required | Disconnect from a provider            |
| POST   | `/api/v1/providers/{id}/refresh`     | Required | Manually refresh provider token       |
| GET    | `/api/v1/services/{id}/requirements` | Required | List service provider requirements    |
| POST   | `/api/v1/services/{id}/requirements` | Admin    | Add a provider requirement            |
| DELETE | `/api/v1/services/{id}/requirements/{rid}` | Admin | Remove a provider requirement    |
| POST   | `/api/v1/mfa/setup`                  | Required | Begin TOTP MFA enrollment            |
| POST   | `/api/v1/mfa/verify-setup`           | Required | Complete TOTP MFA enrollment         |
| GET    | `/api/v1/llm/status`                 | Required | LLM provider readiness per user      |
| ANY    | `/api/v1/llm/{provider_slug}/v1/{*path}` | Required | Proxy to LLM provider           |
| ANY    | `/api/v1/llm/gateway/v1/{*path}`     | Required | OpenAI-compatible LLM gateway        |
| POST   | `/api/v1/delegation/refresh`         | Delegated| Refresh a delegated access token     |
| POST   | `/api/v1/admin/service-accounts`     | Admin    | Create a service account             |
| GET    | `/api/v1/admin/service-accounts`     | Admin    | List service accounts (paginated, searchable) |
| GET    | `/api/v1/admin/service-accounts/{sa_id}` | Admin | Get service account details          |
| PUT    | `/api/v1/admin/service-accounts/{sa_id}` | Admin | Update a service account             |
| DELETE | `/api/v1/admin/service-accounts/{sa_id}` | Admin | Delete (deactivate) a service account |
| POST   | `/api/v1/admin/service-accounts/{sa_id}/rotate-secret` | Admin | Rotate client secret |
| POST   | `/api/v1/admin/service-accounts/{sa_id}/revoke-tokens` | Admin | Revoke all tokens    |

`POST /oauth/token` also supports `grant_type=client_credentials` for service account authentication.

---

## Environment Variables

All configuration is loaded from environment variables. A `.env` file is supported via `dotenvy`.

### Required

| Variable         | Description                                        | Example                                        |
|------------------|----------------------------------------------------|------------------------------------------------|
| `DATABASE_URL`   | MongoDB connection string                          | `mongodb://localhost:27017/nyxid`              |
| `ENCRYPTION_KEY` | 32-byte hex-encoded AES-256 key (64 hex chars)     | Output of `openssl rand -hex 32`               |

### Server

| Variable       | Default                  | Description                          |
|----------------|--------------------------|--------------------------------------|
| `PORT`         | `3001`                   | HTTP listen port                     |
| `BASE_URL`     | `http://localhost:3001`  | Backend base URL (used in JWT `aud`) |
| `FRONTEND_URL` | `http://localhost:3000`  | Frontend origin for CORS             |
| `ENVIRONMENT`  | `development`            | `development`, `staging`, `production` |

### Database

| Variable                   | Default | Description                     |
|----------------------------|---------|---------------------------------|
| `DATABASE_MAX_CONNECTIONS` | `10`    | Connection pool max size        |

### JWT

| Variable               | Default              | Description                              |
|------------------------|----------------------|------------------------------------------|
| `JWT_PRIVATE_KEY_PATH` | `keys/private.pem`   | Path to RSA private key PEM file         |
| `JWT_PUBLIC_KEY_PATH`  | `keys/public.pem`    | Path to RSA public key PEM file          |
| `JWT_ISSUER`           | `nyxid`              | JWT `iss` claim value                    |
| `JWT_ACCESS_TTL_SECS`  | `900` (15 min)       | Access token lifetime in seconds         |
| `JWT_REFRESH_TTL_SECS` | `604800` (7 days)    | Refresh token lifetime in seconds        |
| `SA_TOKEN_TTL_SECS`   | `3600` (1 hour)      | Service account token lifetime in seconds |

In development mode, RSA keys are auto-generated if the files do not exist. In production, you must provide pre-generated keys:

```bash
openssl genrsa -out keys/private.pem 4096
openssl rsa -in keys/private.pem -pubout -out keys/public.pem
chmod 600 keys/private.pem
```

### Rate Limiting

| Variable               | Default | Description                            |
|------------------------|---------|----------------------------------------|
| `RATE_LIMIT_PER_SECOND`| `10`    | Global rate limit (requests/second)    |
| `RATE_LIMIT_BURST`     | `30`    | Burst capacity and per-IP limit        |

### Social Login (Optional)

| Variable               | Description             |
|------------------------|-------------------------|
| `GOOGLE_CLIENT_ID`     | Google OAuth client ID  |
| `GOOGLE_CLIENT_SECRET` | Google OAuth secret     |
| `GITHUB_CLIENT_ID`     | GitHub OAuth client ID  |
| `GITHUB_CLIENT_SECRET` | GitHub OAuth secret     |

### SMTP (Optional)

| Variable            | Description                       |
|---------------------|-----------------------------------|
| `SMTP_HOST`         | SMTP server hostname              |
| `SMTP_PORT`         | SMTP server port                  |
| `SMTP_USERNAME`     | SMTP authentication username      |
| `SMTP_PASSWORD`     | SMTP authentication password      |
| `SMTP_FROM_ADDRESS` | Sender address for outbound email |

For development, Mailpit is provided via Docker Compose (SMTP on `localhost:1025`, web UI at `http://localhost:8025`).

### Logging

| Variable   | Default                                | Description              |
|------------|----------------------------------------|--------------------------|
| `RUST_LOG` | `nyxid=info,tower_http=info` | Tracing filter string |

---

## Database Schema

NyxID uses 19 MongoDB collections:

| Collection                 | Description                                          |
|----------------------------|------------------------------------------------------|
| `users`                    | User accounts (email, password hash, MFA status)     |
| `sessions`                 | Server-side sessions with hashed tokens              |
| `oauth_clients`            | Registered OIDC/OAuth clients (includes `delegation_scopes` for token exchange) |
| `authorization_codes`      | Short-lived OIDC authorization codes                 |
| `refresh_tokens`           | Issued refresh tokens with rotation chain tracking   |
| `api_keys`                 | User-scoped API keys (hashed, with prefix)           |
| `downstream_services`      | Registered downstream services for proxying (includes auto-seeded LLM services via `provider_config_id`, `inject_delegation_token` and `delegation_token_scope` for delegated access) |
| `user_service_connections` | Per-user connections and encrypted credentials for downstream services |
| `mfa_factors`              | TOTP factors and encrypted recovery codes            |
| `service_endpoints`        | Registered API endpoints per service (MCP tools)     |
| `provider_configs`         | External provider registry (encrypted OAuth creds)   |
| `user_provider_tokens`     | Per-user encrypted provider tokens (API keys/OAuth)  |
| `service_provider_requirements` | Provider token requirements per service          |
| `oauth_states`             | Temporary OAuth state for provider flows             |
| `roles`                    | Role definitions with permissions and scoping        |
| `groups`                   | Group definitions with role inheritance               |
| `consents`                 | User OAuth consent records per client                 |
| `service_accounts`         | Non-human (machine) identity definitions             |
| `service_account_tokens`   | Issued service account JWT records for revocation    |
| `audit_log`                | Immutable audit trail of security events             |

All documents use UUID identifiers, ISO 8601 timestamps, and appropriate indexes for query patterns.

For the full schema with fields and relationships, see **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)**.

---

## Security

### Cryptography

| Purpose              | Algorithm / Standard                           |
|----------------------|------------------------------------------------|
| Password hashing     | Argon2id (m=64MiB, t=3, p=4)                  |
| JWT signing          | RS256 with 4096-bit RSA keys                   |
| Encryption at rest   | AES-256-GCM with random 96-bit nonces          |
| Token hashing        | SHA-256                                         |
| PKCE                 | S256 (SHA-256 code challenge)                   |

### HTTP Security Headers

Every response includes:
- `Strict-Transport-Security: max-age=31536000; includeSubDomains; preload`
- `X-Content-Type-Options: nosniff`
- `X-Frame-Options: DENY`
- `Content-Security-Policy: default-src 'none'; frame-ancestors 'none'`
- `Referrer-Policy: strict-origin-when-cross-origin`
- `Permissions-Policy: camera=(), microphone=(), geolocation=(), interest-cohort=()`
- `X-XSS-Protection: 1; mode=block`

### Cookie Security

- All authentication cookies are `HttpOnly` and `SameSite=Lax`
- `Secure` flag is automatically set when not running on localhost
- Refresh tokens are path-scoped to `/api/v1/auth/refresh`

### SSRF Protection

The service registration endpoint validates that `base_url` values:
- Use `https://` or `http://` scheme only
- Do not resolve to private IP ranges (10.x, 172.16-31.x, 192.168.x, 127.x, ::1)
- Do not point to `localhost`, `metadata.google.internal`, or other reserved hosts

### Rate Limiting

Dual-layer rate limiting:
1. **Per-IP**: Sliding window counter per client IP (configurable via `RATE_LIMIT_BURST`)
2. **Global**: Token-bucket algorithm as a safety net for total server throughput

Returns HTTP 429 when limits are exceeded.

---

## MCP Integration

NyxID is designed to be accessible to AI agents via the Model Context Protocol (MCP). A dedicated MCP proxy (`mcp-proxy/`) exposes connected downstream services as MCP tools.

**How it works:**
- MCP sessions start with 3 meta-tools: `nyx__search_tools`, `nyx__discover_services`, and `nyx__connect_service`
- Service tools are loaded on-demand when the LLM calls `nyx__search_tools` or `nyx__connect_service`
- The server sends `notifications/tools/list_changed` so clients automatically refresh their tool lists
- Each service endpoint is mapped to an MCP tool named `{service_slug}__{endpoint_name}`
- Tool calls are forwarded through NyxID's authenticated proxy with per-user credential injection
- Maximum 20 activated services per session to bound memory usage

**Agent capabilities:**
- Authenticate users and manage sessions
- Create and rotate API keys
- Register and query downstream services
- Proxy requests to downstream services on behalf of users
- Query audit logs

This makes NyxID suitable as an identity and credential management layer in agentic workflows. See **[docs/DEPLOYMENT.md](docs/DEPLOYMENT.md)** for MCP proxy deployment instructions.

---

## Development Guide

### Running Tests

```bash
# Backend unit tests
cargo test --manifest-path backend/Cargo.toml

# Frontend lint
cd frontend && npm run lint
```

### Code Organization

The backend follows a strict layered architecture:

- **`handlers/`** -- HTTP request/response logic only. No business logic.
- **`services/`** -- Business logic. No HTTP types.
- **`models/`** -- MongoDB document structs (serde). No logic.
- **`crypto/`** -- Cryptographic operations. Pure functions where possible.
- **`mw/`** -- Axum middleware (auth extraction, rate limiting, security headers).
- **`errors/`** -- Centralized error types with HTTP status code mapping.

### Adding a New Endpoint

1. Define request/response types in `handlers/<module>.rs`
2. Implement business logic in `services/<module>.rs`
3. Register the route in `routes.rs`
4. Add audit logging where appropriate

### Frontend Development

The frontend uses:
- **React 19** with function components and hooks
- **TanStack Router** for type-safe file-based routing
- **TanStack Query** for server state management
- **Zustand** for client-side auth state
- **shadcn/ui** (Radix primitives + Tailwind CSS v4) for the component library
- **Zod v4** for runtime schema validation
- **React Hook Form** with Zod resolvers for form handling

### Production Deployment Checklist

- [ ] Set `ENVIRONMENT=production`
- [ ] Generate and mount RSA key pair (`keys/private.pem`, `keys/public.pem`)
- [ ] Generate a secure `ENCRYPTION_KEY` (`openssl rand -hex 32`)
- [ ] Configure a real `DATABASE_URL` with SSL
- [ ] Set `BASE_URL` and `FRONTEND_URL` to production origins
- [ ] Configure social login provider credentials if needed
- [ ] Configure SMTP for transactional email
- [ ] Place behind a reverse proxy (nginx, Caddy) that sets `X-Forwarded-For`
- [ ] Enable TLS termination at the reverse proxy
- [ ] Set `RUST_LOG=nyxid=info,tower_http=warn` for production log levels

---

## Project Structure

```
NyxID/
|-- Cargo.toml                  Workspace root (backend)
|-- docker-compose.yml          MongoDB 8.0 + Mailpit
|-- .env.example                Environment variable template
|-- .gitignore                  Ignores target/, node_modules/, keys/, .env
|
|-- backend/
|   |-- Cargo.toml              Backend dependencies
|   `-- src/
|       |-- main.rs             Entry point, middleware stack, server startup
|       |-- config.rs           AppConfig loaded from environment variables
|       |-- db.rs               Database connection pool setup
|       |-- routes.rs           Router definition with all route groups
|       |-- errors/mod.rs       AppError enum, error codes, JSON error responses
|       |-- crypto/
|       |   |-- password.rs     Argon2id hashing and verification
|       |   |-- jwt.rs          RS256 JWT signing, verification, key management
|       |   |-- aes.rs          AES-256-GCM encryption and decryption
|       |   `-- token.rs        Random token generation, SHA-256 hashing
|       |-- models/             MongoDB document definitions (22 modules, incl. role, group, consent, service_account)
|       |-- handlers/           HTTP handler functions by domain
|       |   |-- auth.rs         Register, login, logout, refresh, verify-email, forgot/reset-password
|       |   |-- social_auth.rs  Social login: authorize redirect + OAuth callback
|       |   |-- users.rs        Get/update user profile
|       |   |-- api_keys.rs     CRUD + rotate API keys
|       |   |-- services.rs     CRUD downstream services (+ identity propagation config)
|       |   |-- connections.rs  Connect/disconnect, credential management
|       |   |-- providers.rs    CRUD external provider configurations
|       |   |-- user_tokens.rs  User provider token management (API key + OAuth)
|       |   |-- service_requirements.rs  Service provider requirement management
|       |   |-- proxy.rs        Reverse proxy handler (+ identity + delegation)
|       |   |-- llm_gateway.rs  LLM gateway handlers (proxy, gateway, status)
|       |   |-- mcp.rs          MCP config endpoint
|       |   |-- mcp_transport.rs MCP SSE/Streamable HTTP transport
|       |   |-- endpoints.rs    Service endpoint CRUD (MCP tools)
|       |   |-- sessions.rs     Session listing
|       |   |-- oidc_discovery.rs OpenID Connect discovery
|       |   |-- oauth.rs        OIDC authorize, token, userinfo
|       |   |-- admin.rs        Admin user management, audit log, OAuth client endpoints
|       |   |-- admin_roles.rs  Admin role CRUD + user role assignment
|       |   |-- admin_groups.rs Admin group CRUD + membership management
|       |   |-- admin_service_accounts.rs Admin service account CRUD + secret rotation + token revocation
|       |   |-- admin_helpers.rs Shared admin handler helpers (require_admin, IP/UA extraction)
|       |   |-- consent.rs      User consent listing and revocation
|       |   |-- delegation.rs   Delegation token refresh endpoint
|       |   |-- mfa.rs          MFA setup and verification
|       |   `-- health.rs       Health check
|       |-- services/           Business logic layer
|       |   |-- auth_service.rs     User registration, password verification
|       |   |-- social_auth_service.rs Social login OAuth flow (GitHub + Google)
|       |   |-- token_service.rs    Session/token issuance, refresh rotation
|       |   |-- oauth_service.rs    Client validation, code exchange
|       |   |-- key_service.rs      API key lifecycle
|       |   |-- proxy_service.rs    Target resolution, request forwarding (+ identity + delegation)
|       |   |-- connection_service.rs Connection lifecycle, credential management
|       |   |-- provider_service.rs Provider registry CRUD, encrypted credential storage
|       |   |-- user_token_service.rs User provider token lifecycle (API key + OAuth)
|       |   |-- delegation_service.rs Credential delegation resolution for proxy
|       |   |-- token_exchange_service.rs RFC 8693 Token Exchange for delegated access
|       |   |-- llm_gateway_service.rs LLM gateway: model routing, format translation
|       |   |-- identity_service.rs Identity propagation headers + JWT assertions
|       |   |-- oauth_flow.rs       OAuth2 utilities (PKCE, token exchange, refresh)
|       |   |-- mfa_service.rs      TOTP provisioning, verification
|       |   |-- admin_user_service.rs Admin user CRUD, cascade delete, session revocation
|       |   |-- role_service.rs     Role CRUD, assignment, system role seeding
|       |   |-- group_service.rs    Group CRUD, membership management
|       |   |-- consent_service.rs  Consent creation, listing, revocation
|       |   |-- service_account_service.rs Service account CRUD, client credentials auth, token revocation
|       |   |-- rbac_helpers.rs     Resolve effective roles/groups/permissions for a user
|       |   |-- mcp_service.rs      MCP tool execution, delegation token injection
|       |   |-- oauth_client_service.rs OAuth client management (admin)
|       |   |-- service_endpoint_service.rs Service endpoint CRUD
|       |   `-- audit_service.rs    Async audit log insertion
|       `-- mw/                 Middleware
|           |-- auth.rs         AuthUser extractor (Bearer / cookie / API key)
|           |-- rate_limit.rs   Per-IP + global rate limiting
|           `-- security_headers.rs  HSTS, CSP, XFO, etc.
|
`-- frontend/
    |-- package.json            React 19, TanStack, Zustand, shadcn/ui, Zod 4
    |-- vite.config.ts          Vite 7.3 with React plugin + Tailwind
    `-- src/
        |-- main.tsx            Application entry point
        |-- router.tsx          TanStack Router configuration
        |-- lib/                API client, utilities
        |-- stores/             Zustand auth state store
        |-- types/              TypeScript API type definitions
        |   |-- api.ts
        |   |-- admin.ts       Admin-specific types
        |   |-- rbac.ts        RBAC types (roles, groups, consents)
        |   `-- service-accounts.ts Service account types
        |-- schemas/            Zod validation schemas
        |   |-- admin.ts       Admin form schemas
        |   |-- rbac.ts        RBAC form schemas (role, group)
        |   `-- service-accounts.ts Service account form schemas
        |-- hooks/              React Query hooks
        |   |-- use-admin.ts   Admin user management hooks
        |   |-- use-rbac.ts    Role and group management hooks
        |   |-- use-consents.ts Consent management hooks
        |   |-- use-service-accounts.ts Service account management hooks
        |   `-- use-llm-gateway.ts LLM gateway status hook
        |-- components/
        |   |-- ui/             16 shadcn/ui primitives
        |   |-- auth/           Login, register, MFA forms
        |   |-- dashboard/      Sidebar, header, tables, cards
        |   `-- layout/         Auth and dashboard layout shells
        `-- pages/              Route pages
            |-- admin-roles.tsx    Admin role list
            |-- admin-role-detail.tsx  Admin role detail
            |-- admin-groups.tsx   Admin group list
            |-- admin-group-detail.tsx Admin group detail with member management
            |-- admin-service-accounts.tsx Admin service account list
            |-- admin-service-account-detail.tsx Admin service account detail
            |-- consents.tsx       User consent management
            `-- (login, register, dashboard, admin-users, admin-user-detail, etc.)
```

---

## License

MIT License. See [LICENSE](LICENSE) for details.
