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
- Google, GitHub, and Apple OAuth 2.0 integration
- Automatic account linking by email
- Encrypted storage of provider tokens (AES-256-GCM)

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
- User listing with pagination
- Per-user detail views
- Audit log with action, resource, IP, and user-agent tracking
- Admin-only access control

### Credential Broker
- Admin-managed provider registry (OpenAI, Anthropic, Google AI, Mistral, Cohere, etc.)
- Users connect by entering API keys or completing OAuth2 flows
- All credentials encrypted at rest (AES-256-GCM) with secure memory cleanup (zeroize)
- Credential delegation: downstream services declare provider requirements, proxy injects user tokens automatically
- Lazy OAuth token refresh with 5-minute buffer before expiry
- Token lifecycle tracking: active, expired, revoked, refresh_failed

### Identity Propagation
- Forward authenticated user identity to downstream services during proxy requests
- Four modes: `none`, `headers`, `jwt`, `both`
- Header mode: `X-NyxID-User-Id`, `X-NyxID-User-Email`, `X-NyxID-User-Name`
- JWT mode: Short-lived RS256-signed identity assertion (60-second TTL) via `X-NyxID-Identity-Token`
- Per-service configuration of which claims to include
- CRLF injection prevention on all header values

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
                         |  |  proxy        |  ANY  /api/v1/proxy/:id/*
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
                         |  (14 collections)|
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
| GET    | `/oauth/authorize`                   | Required | OIDC authorization endpoint          |
| POST   | `/oauth/token`                       | None     | OIDC token endpoint                  |
| GET    | `/oauth/userinfo`                    | Required | OIDC userinfo endpoint               |
| GET    | `/api/v1/admin/users`                | Admin    | List all users (paginated)           |
| GET    | `/api/v1/admin/users/{user_id}`      | Admin    | Get user details                     |
| GET    | `/api/v1/admin/audit-log`            | Admin    | Query audit log (paginated)          |
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

NyxID uses 14 MongoDB collections:

| Collection                 | Description                                          |
|----------------------------|------------------------------------------------------|
| `users`                    | User accounts (email, password hash, MFA status)     |
| `sessions`                 | Server-side sessions with hashed tokens              |
| `oauth_clients`            | Registered OIDC/OAuth clients                        |
| `authorization_codes`      | Short-lived OIDC authorization codes                 |
| `refresh_tokens`           | Issued refresh tokens with rotation chain tracking   |
| `api_keys`                 | User-scoped API keys (hashed, with prefix)           |
| `downstream_services`      | Registered downstream services for proxying          |
| `user_service_connections` | Per-user connections and encrypted credentials for downstream services |
| `mfa_factors`              | TOTP factors and encrypted recovery codes            |
| `service_endpoints`        | Registered API endpoints per service (MCP tools)     |
| `provider_configs`         | External provider registry (encrypted OAuth creds)   |
| `user_provider_tokens`     | Per-user encrypted provider tokens (API keys/OAuth)  |
| `service_provider_requirements` | Provider token requirements per service          |
| `oauth_states`             | Temporary OAuth state for provider flows             |
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
- Each downstream service endpoint is mapped to an MCP tool named `{service_slug}__{endpoint_name}`
- The proxy authenticates via NyxID's OAuth endpoints and fetches the user's MCP config
- Tool calls are forwarded through NyxID's authenticated proxy with per-user credential injection
- When a user has more than 20 connected tools, a built-in `nyxid__search_tools` meta-tool is added for discovery
- Only services with valid connections and satisfied credentials appear as available tools

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
|       |-- models/             MongoDB document definitions (10 modules)
|       |-- handlers/           HTTP handler functions by domain
|       |   |-- auth.rs         Register, login, logout, refresh, verify-email, forgot/reset-password
|       |   |-- users.rs        Get/update user profile
|       |   |-- api_keys.rs     CRUD + rotate API keys
|       |   |-- services.rs     CRUD downstream services (+ identity propagation config)
|       |   |-- connections.rs  Connect/disconnect, credential management
|       |   |-- providers.rs    CRUD external provider configurations
|       |   |-- user_tokens.rs  User provider token management (API key + OAuth)
|       |   |-- service_requirements.rs  Service provider requirement management
|       |   |-- proxy.rs        Reverse proxy handler (+ identity + delegation)
|       |   |-- mcp.rs          MCP config endpoint
|       |   |-- oauth.rs        OIDC authorize, token, userinfo
|       |   |-- admin.rs        Admin user/audit endpoints
|       |   |-- mfa.rs          MFA setup and verification
|       |   `-- health.rs       Health check
|       |-- services/           Business logic layer
|       |   |-- auth_service.rs     User registration, password verification
|       |   |-- token_service.rs    Session/token issuance, refresh rotation
|       |   |-- oauth_service.rs    Client validation, code exchange
|       |   |-- key_service.rs      API key lifecycle
|       |   |-- proxy_service.rs    Target resolution, request forwarding (+ identity + delegation)
|       |   |-- connection_service.rs Connection lifecycle, credential management
|       |   |-- provider_service.rs Provider registry CRUD, encrypted credential storage
|       |   |-- user_token_service.rs User provider token lifecycle (API key + OAuth)
|       |   |-- delegation_service.rs Credential delegation resolution for proxy
|       |   |-- identity_service.rs Identity propagation headers + JWT assertions
|       |   |-- oauth_flow.rs       OAuth2 utilities (PKCE, token exchange, refresh)
|       |   |-- mfa_service.rs      TOTP provisioning, verification
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
        |-- schemas/            Zod validation schemas
        |-- hooks/              React Query hooks
        |-- components/
        |   |-- ui/             16 shadcn/ui primitives
        |   |-- auth/           Login, register, MFA forms
        |   |-- dashboard/      Sidebar, header, tables, cards
        |   `-- layout/         Auth and dashboard layout shells
        `-- pages/              Route pages (login, register, dashboard, etc.)
```

---

## License

MIT License. See [LICENSE](LICENSE) for details.
