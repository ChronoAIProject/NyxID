## Project Overview

NyxID is an Auth/SSO platform (similar to Supabase Auth) with a Rust backend and React frontend. It provides user authentication, OAuth/OIDC, MFA, credential brokering, admin management, and MCP proxy capabilities.

**Tech Stack:**
- **Backend:** Rust, Axum 0.8, MongoDB 8.0 (`mongodb` 3.5, `bson` 2.15)
- **Frontend:** React 19, TypeScript, Vite 7, TanStack Router + Query, Tailwind CSS 4, Zod 4, Zustand
- **Dev tools:** Docker Compose (MongoDB + Mailpit), RSA keys for JWT signing

## Critical Rules

### 1. MongoDB Model Conventions

- NEVER use `#[serde(skip_serializing)]` on model fields -- prevents `insert_one(&struct)` from storing them
- ALWAYS use `#[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]` on `DateTime<Utc>` fields
- For `Option<DateTime<Utc>>`, use the custom `bson_datetime::optional` helper (in `models/bson_datetime.rs`)
- IDs are UUID v4 stored as strings in MongoDB `_id` fields
- Each model has a `COLLECTION_NAME` constant

### 2. Layer Architecture

Strict separation: `handlers/` -> `services/` -> `models/`
- **models/** -- Plain structs with serde, `COLLECTION_NAME` constant, no business logic
- **services/** -- Business logic, takes `&mongodb::Database` and `&str` for IDs
- **handlers/** -- HTTP layer, converts `AuthUser.user_id` (Uuid) to string for services, uses dedicated response structs (never serialize model structs to API responses)
- **crypto/jwt.rs** -- JWT functions take `&Uuid` (kept for signing)
- **token_service** -- Parses `&str` to `Uuid` internally for JWT generation

### 3. Error Handling

Backend uses `AppError` enum (`errors/mod.rs`) with `thiserror`:
```rust
fn my_handler() -> AppResult<Json<MyResponse>> {
    // AppResult<T> = Result<T, AppError>
}
```
Error variants map to HTTP status codes and numeric error codes (1000-3002). Internal/database errors never leak details to clients.

### 4. Frontend Patterns

- Validation with Zod schemas (`schemas/` directory, one per domain)
- React Hook Form with `@hookform/resolvers` for form handling
- TanStack Query hooks in `hooks/` (one per domain: `use-auth.ts`, `use-services.ts`, etc.)
- Auth state in Zustand store (`stores/auth-store.ts`)
- UI components via Radix UI + shadcn/ui pattern (`components/ui/`)
- No `console.log` in production code

### 5. Security

- No hardcoded secrets -- environment variables for all sensitive data
- AES-256 encryption for stored credentials (`crypto/aes.rs`)
- Rate limiting middleware (`mw/rate_limit.rs`)
- Security headers middleware (`mw/security_headers.rs`)
- JWT auth middleware (`mw/auth.rs`)
- PKCE for OAuth flows
- Input validation on all endpoints

## File Structure

```
backend/src/
|-- config.rs            # AppConfig from env vars
|-- db.rs                # MongoDB connection + ensure_indexes()
|-- routes.rs            # All route definitions
|-- main.rs              # Server startup
|-- models/              # MongoDB document structs (25 models, 23 collections)
|-- services/            # Business logic (29 services, incl. approval_service, notification_service, telegram_service, social_token_exchange_service)
|-- handlers/            # HTTP handlers (29 handler modules, incl. approvals, notifications, webhooks)
|-- crypto/              # JWT, AES, password hashing, token generation, JWKS cache (jwks.rs)
|-- errors/              # AppError enum, ErrorResponse, AppResult
|-- mw/                  # Middleware: auth, rate_limit, security_headers

frontend/src/
|-- pages/               # Route pages (23 pages, incl. approval-history, approval-grants, notification-settings)
|-- components/          # UI components (auth/, dashboard/, layout/, shared/, ui/)
|-- hooks/               # TanStack Query hooks (9 hooks, incl. use-approvals)
|-- schemas/             # Zod validation schemas (7 schema files + tests)
|-- stores/              # Zustand stores (auth-store)
|-- lib/                 # API client, constants, utils
|-- types/               # TypeScript type definitions
|-- router.tsx           # TanStack Router config
```

## Key API Routes

All API routes under `/api/v1`:
- `/auth` -- register, login, logout, refresh, verify-email, forgot/reset-password
- `/users` -- get/update current user
- `/mfa` -- setup, verify-setup
- `/api-keys` -- CRUD + rotate
- `/services` -- CRUD + OIDC credentials + endpoints + requirements
- `/sessions` -- list sessions
- `/connections` -- connect/disconnect services
- `/providers` -- CRUD + OAuth/device-code/API-key flows + token management
- `/admin` -- user management, audit log, OAuth clients, service accounts
- `/proxy/{service_id}/{path}` -- authenticated proxy (UUID-based)
- `/proxy/s/{slug}/{path}` -- authenticated proxy (slug-based)
- `/proxy/services` -- service discovery (paginated list of proxyable services)
- `/llm` -- LLM gateway (provider proxy, OpenAI-compatible gateway, status)
- `/delegation/refresh` -- refresh delegated access tokens
- `/notifications` -- notification settings CRUD, Telegram link/disconnect
- `/approvals` -- approval request history, grants, decide, status polling, per-service approval configs
- `/webhooks/telegram` -- Telegram webhook (unauthenticated, secret-verified)

- `/admin/service-accounts` -- service account CRUD, secret rotation, token revocation, provider management (connect via API key/OAuth redirect/device-code, list, disconnect providers on behalf of SAs)

- `/oauth/token` -- also supports `grant_type=client_credentials` (service accounts), `grant_type=urn:ietf:params:oauth:grant-type:token-exchange` (RFC 8693 delegated access and social token exchange via `subject_token_type=id_token` for native mobile Google/GitHub login)

Top-level: `/health`, `/.well-known/openid-configuration`, `/oauth/*`, `/mcp`

## Environment Variables

```bash
# Required
DATABASE_URL=mongodb://...          # MongoDB connection string
ENCRYPTION_KEY=                     # 64 hex chars (32 bytes AES-256)

# Defaults provided
PORT=3001
BASE_URL=http://localhost:3001
FRONTEND_URL=http://localhost:3000
JWT_PRIVATE_KEY_PATH=keys/private.pem
JWT_PUBLIC_KEY_PATH=keys/public.pem
JWT_ISSUER=nyxid
JWT_ACCESS_TTL_SECS=900             # 15 minutes
JWT_REFRESH_TTL_SECS=604800         # 7 days
SA_TOKEN_TTL_SECS=3600              # 1 hour (service account tokens)
ENVIRONMENT=development
RATE_LIMIT_PER_SECOND=10
RATE_LIMIT_BURST=30

# Telegram / Approval System (optional)
TELEGRAM_BOT_TOKEN=                     # From @BotFather
TELEGRAM_WEBHOOK_SECRET=                # Random string for webhook verification
TELEGRAM_WEBHOOK_URL=                   # e.g. https://auth.nyxid.dev/api/v1/webhooks/telegram
TELEGRAM_BOT_USERNAME=                  # Bot username without @
APPROVAL_EXPIRY_INTERVAL_SECS=5         # Interval between expiry sweeps

# Optional
GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET
GITHUB_CLIENT_ID / GITHUB_CLIENT_SECRET
SMTP_HOST / SMTP_PORT / SMTP_USERNAME / SMTP_PASSWORD / SMTP_FROM_ADDRESS
```

## Available Commands

```bash
# Backend (from project root)
source "$HOME/.cargo/env" 2>/dev/null  # Ensure cargo is available
cargo build                             # Build backend
cargo test                              # Run backend tests
cargo run                               # Start backend (port 3001)

# Frontend (from frontend/)
npm run dev                             # Dev server (port 3000)
npm run build                           # Type-check + production build
npm run test                            # Run vitest
npm run test:watch                      # Vitest in watch mode
npm run lint                            # ESLint

# Docker (from project root)
docker compose up -d                    # Start MongoDB (27018) + Mailpit (8025)
```

## Git Workflow

- Conventional commits: `feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`
- Never commit to main directly
- PRs require review
- All tests must pass before merge
