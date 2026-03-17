## Project Overview

NyxID is an Auth/SSO platform (similar to Supabase Auth) with a Rust backend and React frontend. It provides user authentication, OAuth/OIDC, MFA, credential brokering, admin management, and MCP proxy capabilities.

**Tech Stack:**
- **Backend:** Rust, Axum 0.8, MongoDB 8.0 (`mongodb` 3.5, `bson` 2.15)
- **Frontend:** React 19, TypeScript, Vite 7, TanStack Router + Query, Tailwind CSS 4, Zod 4, Zustand
- **Mobile:** React Native 0.79, Expo 53, TypeScript (iOS + Android approval app)
- **SDK:** TypeScript OAuth 2.0 client (`@nyxids/oauth-core`, `@nyxids/oauth-react`)
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
Error variants map to HTTP status codes and numeric error codes (1000-3002, 7000, 8000-8003). Internal/database errors never leak details to clients.

### 4. Frontend Patterns

- Validation with Zod schemas (`schemas/` directory, one per domain)
- React Hook Form with `@hookform/resolvers` for form handling
- TanStack Query hooks in `hooks/` (one per domain: `use-auth.ts`, `use-services.ts`, etc.)
- Auth state in Zustand store (`stores/auth-store.ts`)
- UI components via Radix UI + shadcn/ui pattern (`components/ui/`)
- No `console.log` in production code

### 5. Security

- No hardcoded secrets -- environment variables for all sensitive data
- AES-256 envelope encryption with pluggable async `KeyProvider` trait (`crypto/aes.rs`, `crypto/key_provider.rs`)
- Cloud KMS support: AWS KMS (`crypto/aws_kms_provider.rs`, feature `aws-kms`) and GCP Cloud KMS (`crypto/gcp_kms_provider.rs`, feature `gcp-kms`) behind feature flags
- Fallback provider for zero-downtime migration between encryption backends
- All key material in `Zeroizing` wrappers; all Debug impls redact secrets and key identifiers
- `MAX_WRAPPED_DEK_SIZE = 1024` enforced on encrypt and decrypt paths
- Rate limiting middleware (`mw/rate_limit.rs`)
- Security headers middleware (`mw/security_headers.rs`)
- JWT auth middleware (`mw/auth.rs`)
- PKCE for OAuth flows
- Input validation on all endpoints

### 6. Node Proxy Conventions

- `NodeWsManager` is an in-memory connection pool shared via `Arc` in `AppState`; uses `DashMap` for lock-free concurrent access
- Node auth tokens (`nyx_nauth_...`) and registration tokens (`nyx_nreg_...`) are 32-byte random values; only SHA-256 hashes are stored
- HMAC signing secrets are generated at registration; stored as SHA-256 hashes on server, encrypted locally on the node agent
- WebSocket handler (`handlers/node_ws.rs`) authenticates in the first message, not via HTTP middleware
- Proxy routing check (`node_routing_service::resolve_node_route`) runs before credential resolution in `execute_proxy()`; returns `NodeRoute` with `fallback_node_ids` for multi-node failover
- Streaming proxy uses `proxy_response_start` / `proxy_response_chunk` / `proxy_response_end` messages; `PendingRequest` upgrades from `OneShot` to `Streaming` on first chunk
- Node metrics (`node_metrics_service`) are recorded asynchronously (fire-and-forget) after each proxy request; stored as embedded `NodeMetrics` document on the Node model
- Node-routed audit events include `"routed_via": "node"` and `"node_id"` in event data
- Error codes 8000-8003 are reserved for node errors (`NodeNotFound`, `NodeOffline`, `NodeProxyTimeout`, `NodeRegistrationFailed`)
- `NodeStatus` is an enum (`Online`/`Offline`/`Draining`) -- not a bare string
- WS writer channels are bounded (capacity: 256); `try_send` treats full buffers as node offline (H4)
- Admin node endpoints (`handlers/admin_nodes.rs`) require admin role and have no ownership check

## File Structure

```
node-agent/src/
|-- main.rs              # CLI entry point, command dispatch (register, start, status, credentials, version)
|-- cli.rs               # Clap subcommand definitions
|-- config.rs            # TOML config (server url, node id, encrypted auth token, signing secret, credentials)
|-- ws_client.rs         # WebSocket connection loop, exponential backoff reconnection, graceful shutdown
|-- proxy_executor.rs    # HTTP request execution, credential injection, SSE streaming detection
|-- credential_store.rs  # In-memory decrypted credential store (header or query_param injection)
|-- signing.rs           # HMAC-SHA256 verification, replay guard (5min skew, 10k nonce cap)
|-- metrics.rs           # Local atomic counters (total_requests, success_count, error_count)
|-- encryption.rs        # AES-256-GCM local encryption, keyfile management (0600 mode)
|-- keychain.rs          # OS keychain storage backend (macOS/Windows/Linux)
|-- secret_backend.rs    # Pluggable secret storage trait (file vs keychain)
|-- error.rs             # Error enum with thiserror

backend/src/
|-- config.rs            # AppConfig from env vars
|-- db.rs                # MongoDB connection + ensure_indexes()
|-- routes.rs            # All route definitions
|-- main.rs              # Server startup
|-- models/              # MongoDB document structs (31 models, 29 collections, incl. node, node_service_binding, mcp_session)
|-- services/            # Business logic (37 services, incl. node_service, node_routing_service, node_ws_manager, node_metrics_service)
|-- handlers/            # HTTP handlers (37 handler modules, incl. node_admin, admin_nodes, node_ws, developer_apps)
|-- crypto/              # JWT, AES, password hashing, token generation, KeyProvider trait, KMS providers, JWKS
|-- errors/              # AppError enum, ErrorResponse, AppResult
|-- mw/                  # Middleware: auth, rate_limit, security_headers

frontend/src/
|-- pages/               # Route pages (38 pages, incl. nodes, node-detail, admin-nodes, service-detail, providers)
|-- components/          # UI components (auth/, dashboard/, layout/, shared/, ui/)
|-- hooks/               # TanStack Query hooks (15 hooks, incl. use-nodes, use-admin-nodes, use-providers, use-developer-apps)
|-- schemas/             # Zod validation schemas (8 schema files + tests, incl. nodes.ts)
|-- stores/              # Zustand stores (auth-store)
|-- lib/                 # API client, constants, utils
|-- types/               # TypeScript type definitions (6 files, incl. AdminNodeInfo, NodeMetricsInfo, approvals)
|-- router.tsx           # TanStack Router config

mobile/src/              # React Native + Expo mobile app (Expo 53, RN 0.79, TypeScript)
|-- app/                 # App shell, navigator, deep linking (nyxid://challenge/{id})
|-- features/            # Feature modules: auth, challenges, approvals, account, legal
|-- components/          # Reusable mobile UI components
|-- lib/                 # API client, auth session store (SecureStore), push notification registration
|-- theme/               # Design tokens and mobile theme

sdk/                     # OAuth SDK monorepo (TypeScript, @nyxids/* npm namespace)
|-- oauth-core/          # @nyxids/oauth-core: PKCE OAuth 2.0 client (NyxIDClient class)
|-- oauth-react/         # @nyxids/oauth-react: React context + useNyxID() hook
|-- demo-react/          # Demo Vite app (private, not published)
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
- `/providers` -- CRUD + OAuth/device-code/API-key flows + token management + per-user credentials
- `/admin` -- user management, audit log, OAuth clients, service accounts
- `/proxy/{service_id}/{path}` -- authenticated proxy (UUID-based)
- `/proxy/s/{slug}/{path}` -- authenticated proxy (slug-based)
- `/proxy/services` -- service discovery (paginated list of proxyable services)
- `/llm` -- LLM gateway (provider proxy, OpenAI-compatible gateway, status)
- `/delegation/refresh` -- refresh delegated access tokens
- `/notifications` -- notification settings CRUD, Telegram link/disconnect, device token management (register/list/remove)
- `/approvals` -- approval request history, grants, decide, status polling, per-service approval configs
- `/webhooks/telegram` -- Telegram webhook (unauthenticated, secret-verified)
- `/nodes` -- node management (register-token, list, get, delete, rotate-token, bindings CRUD + priority update)
- `/nodes/ws` -- WebSocket upgrade for node agent connections (auth via WS protocol, not middleware)
- `/admin/nodes` -- admin node management (list all, get, disconnect, delete -- no ownership check)

- `/admin/service-accounts` -- service account CRUD, secret rotation, token revocation, provider management (connect via API key/OAuth redirect/device-code, list, disconnect providers on behalf of SAs)

- `/oauth/token` -- also supports `grant_type=client_credentials` (service accounts), `grant_type=urn:ietf:params:oauth:grant-type:token-exchange` (RFC 8693 delegated access and social token exchange via `subject_token_type=id_token` for native mobile Google/GitHub login)

Top-level: `/health`, `/.well-known/openid-configuration`, `/oauth/*`, `/mcp`

## Environment Variables

```bash
# Required
DATABASE_URL=mongodb://...          # MongoDB connection string
ENCRYPTION_KEY=                     # 64 hex chars (32 bytes AES-256); required for local, optional for KMS (enables fallback)
ENCRYPTION_KEY_PREVIOUS=            # Optional: previous key for zero-downtime rotation (64 hex chars)
KEY_PROVIDER=local                  # Key provider backend: "local" (default), "aws-kms" (feature aws-kms), "gcp-kms" (feature gcp-kms)

# AWS KMS (optional, requires --features aws-kms)
AWS_KMS_KEY_ARN=                    # Full ARN of AWS KMS key (required when KEY_PROVIDER=aws-kms)
AWS_KMS_KEY_ARN_PREVIOUS=           # Optional: previous AWS KMS key ARN for rotation

# GCP Cloud KMS (optional, requires --features gcp-kms)
GCP_KMS_KEY_NAME=                   # Full GCP KMS key resource name (required when KEY_PROVIDER=gcp-kms)
GCP_KMS_KEY_NAME_PREVIOUS=          # Optional: previous GCP KMS key name for rotation

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

# Mobile Push Notifications (optional)
FCM_SERVICE_ACCOUNT_PATH=               # Path to Firebase service account JSON
APNS_KEY_PATH=                          # Path to APNs .p8 private key
APNS_KEY_ID=                            # APNs Key ID (Apple Developer portal)
APNS_TEAM_ID=                           # APNs Team ID (Apple Developer portal)
APNS_TOPIC=                             # APNs topic / iOS bundle ID (e.g. dev.nyxid.app)
APNS_SANDBOX=true                       # Use APNs sandbox (default: true in dev)

# Credential Nodes (optional, all have defaults)
NODE_HEARTBEAT_INTERVAL_SECS=30        # Heartbeat ping interval (default: 30)
NODE_HEARTBEAT_TIMEOUT_SECS=90         # Mark offline after N seconds without heartbeat (default: 90)
NODE_PROXY_TIMEOUT_SECS=30             # Timeout for proxy requests through nodes (default: 30)
NODE_REGISTRATION_TOKEN_TTL_SECS=3600  # Registration token validity (default: 1 hour)
NODE_MAX_PER_USER=10                   # Maximum nodes per user (default: 10)
NODE_MAX_WS_CONNECTIONS=100            # Maximum concurrent node WebSocket connections (default: 100)
NODE_MAX_STREAM_DURATION_SECS=300      # Maximum duration for streaming proxy responses (default: 300)
NODE_HMAC_SIGNING_ENABLED=true         # Enable HMAC request signing for node proxy (default: true)

# Optional
GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET
GITHUB_CLIENT_ID / GITHUB_CLIENT_SECRET
SMTP_HOST / SMTP_PORT / SMTP_USERNAME / SMTP_PASSWORD / SMTP_FROM_ADDRESS
```

## Available Commands

```bash
# Backend (from project root)
source "$HOME/.cargo/env" 2>/dev/null  # Ensure cargo is available
cargo build                             # Build backend (local provider only)
cargo build --features aws-kms          # Build with AWS KMS support
cargo build --features gcp-kms          # Build with GCP Cloud KMS support
cargo build --features aws-kms,gcp-kms  # Build with both KMS providers
cargo test                              # Run backend tests
cargo test --all-features               # Run all tests including KMS provider tests
cargo run                               # Start backend (port 3001)

# Node Agent (from project root)
cargo build -p nyxid-node               # Build node agent binary
cargo test -p nyxid-node                # Run node agent tests
cargo run -p nyxid-node -- register --token nyx_nreg_... --url ws://localhost:3001/api/v1/nodes/ws
cargo run -p nyxid-node -- start        # Start node agent
cargo run -p nyxid-node -- status       # Show node status
cargo run -p nyxid-node -- credentials list  # List configured credentials

# Frontend (from frontend/)
npm run dev                             # Dev server (port 3000)
npm run build                           # Type-check + production build
npm run test                            # Run vitest
npm run test:watch                      # Vitest in watch mode
npm run lint                            # ESLint

# Mobile (from mobile/)
npm run start                           # Expo dev server
npm run ios                             # Run on iOS simulator
npm run android                         # Run on Android emulator

# SDK (from sdk/)
npm run build                           # Build all SDK packages
npm run clean                           # Clean build artifacts

# Docker (from project root)
docker compose up -d                    # Start MongoDB (27018) + Mailpit (8025)
```

## Git Workflow

- Conventional commits: `feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`
- Never commit to main directly
- PRs require review
- All tests must pass before merge
