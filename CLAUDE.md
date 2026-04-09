## Project Overview

NyxID is an Auth/SSO platform (similar to Supabase Auth) with a Rust backend, React frontend, and CLI tools. It provides user authentication, OAuth/OIDC, MFA, credential brokering, admin management, and MCP proxy capabilities. The `nyxid` CLI covers all user-facing operations (services, keys, catalog, nodes, approvals, SSH, MCP, notifications) and also includes the `nyxid node` subcommand for managing on-premise credential nodes.

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
- **unified_key_service** -- Orchestration layer for the streamlined services architecture; auto-provisions UserEndpoint + UserApiKey + UserService from catalog or custom input in a single operation

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
- TanStack Query hooks in `hooks/` (one per domain: `use-auth.ts`, `use-services.ts`, `use-keys.ts`, etc.)
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
- Streaming proxy uses `proxy_response_start` / chunk frames / `proxy_response_end`; preferred chunk transport is a WebSocket binary frame with a 36-byte request_id prefix, with legacy `proxy_response_chunk` JSON fallback for older servers
- Node metrics (`node_metrics_service`) are recorded asynchronously (fire-and-forget) after each proxy request; stored as embedded `NodeMetrics` document on the Node model
- Node-routed audit events include `"routed_via": "node"` and `"node_id"` in event data
- Error codes 8000-8003 are reserved for node errors (`NodeNotFound`, `NodeOffline`, `NodeProxyTimeout`, `NodeRegistrationFailed`)
- `NodeStatus` is an enum (`Online`/`Offline`/`Draining`) -- not a bare string
- WS writer channels are bounded (capacity: 256); `try_send` treats full buffers as node offline (H4)
- Admin node endpoints (`handlers/admin_nodes.rs`) require admin role and have no ownership check
- `nyxid node daemon` subcommands manage background service lifecycle (`cli/src/node/daemon.rs`): `install` creates a launchd LaunchAgent on macOS or systemd user unit on Linux; `start`/`stop`/`restart`/`status`/`logs`/`uninstall` wrap platform service managers
- All node commands support `--profile` for multi-instance operation. Profile-aware service labels: `dev.nyxid.node.{profile}` (macOS) / `nyxid-node-{profile}.service` (Linux). Config stored at `~/.nyxid-node/profiles/{name}/`

### 7. OpenClaw Integration

OpenClaw is a self-hosted AI gateway that NyxID integrates with at three levels:

**Provider (Option 1):** OpenClaw is seeded as an `api_key` provider with `requires_gateway_url: true`. Users connect by providing their gateway URL + bearer token. The `UserProviderToken.gateway_url` field stores the per-user instance URL. `proxy_service::resolve_gateway_url_override()` resolves this URL at proxy time, overriding the service's default `base_url`.

**Node Proxy (Option 2):** The node agent has an `openclaw` subcommand that stores credentials locally, registers the provider connection with NyxID, and creates the node service binding in one step. Proxy requests flow: NyxID -> node agent (WS) -> local OpenClaw instance. The node agent injects the bearer token via the credential store.

**Channel Integration (Option 3):** `openclaw_channel_service` handles inbound webhook messages from OpenClaw channels (WhatsApp, Telegram, Discord, etc.). `openclaw_channel_mappings` collection maps channel users to NyxID users. Each mapping has its own per-user webhook secret (SHA-256 hash stored, raw secret returned once at creation). No server-level env var needed.

Key files:
- `services/openclaw_channel_service.rs` -- HMAC verification, user mapping, provider slug resolution
- `handlers/openclaw_channel.rs` -- Webhook endpoint + mapping CRUD
- `models/user_provider_token.rs` -- `gateway_url` field for per-user instance URLs
- `models/provider_config.rs` -- `requires_gateway_url` flag for self-hosted providers
- `node-agent/src/main.rs` -- `cmd_openclaw_connect/status/disconnect` commands

### 8. Streamlined Services Architecture

The services/connections/providers system was unified into 3 user-managed collections with a single orchestration layer. Old collections are kept for backward compatibility during migration.

**New user collections:**
- `user_endpoints` -- target URLs (custom or auto-provisioned from catalog)
- `user_api_keys` -- external credentials (API keys, OAuth tokens, bearer tokens)
- `user_services` -- proxy routing config (binds endpoint + key + auth method + optional node)

**Orchestration:** `unified_key_service` auto-provisions all 3 records from a single `POST /api/v1/keys` request, using catalog defaults or user-provided values.

**Proxy resolution:** New path checks `UserService` first (by slug + user_id), falls back to old `DownstreamService` + `UserProviderToken` path for unmigrated users.

**ApiKey scope fields** (absorbed from deleted `AgentGroup` model): `allowed_service_ids`, `allowed_node_ids`, `allow_all_services`, `allow_all_nodes`. Enforced at proxy time via `key_service`.

**Frontend:** Unified "AI Services" page at `/keys` with 2 tabs: External Services (UserEndpoint + UserApiKey + UserService) and NyxID API Keys (ApiKey with scope). Services/Connections/Providers removed from normal user sidebar (admin-only). Old `/api-keys` page deleted.

**Old models kept for migration:** DownstreamService (now serves as read-only catalog), UserServiceConnection, UserProviderToken, UserProviderCredentials, NodeServiceBinding (node routing absorbed into `UserService.node_id`).

Key files:
- `services/unified_key_service.rs` -- orchestration: auto-provision endpoint + key + service
- `services/catalog_service.rs` -- read-only catalog from DownstreamService, `list_catalog_all` for full discovery
- `handlers/keys.rs` -- `/api/v1/keys` CRUD
- `handlers/catalog.rs` -- `/api/v1/catalog` read-only, `/api/v1/catalog/{slug}/endpoints` for OpenAPI endpoint discovery
- `models/user_endpoint.rs`, `models/user_api_key.rs`, `models/user_service.rs` -- new user collections

### 9. Agent Isolation

Per-agent credential binding, rate limiting, and audit attribution for AI agents. Each agent (Claude Code, Codex, OpenClaw, etc.) uses its own scoped API key (`nyxid_ag_` prefix) for isolation.

**Backend:**
- `AuthUser` carries `api_key_id`, `api_key_name`, `rate_limit_per_second`, `rate_limit_burst` when auth is via API key
- `ApiKey` model has `rate_limit_per_second`, `rate_limit_burst`, `platform` fields
- `AuditLog` model has `api_key_id`, `api_key_name` for per-agent attribution
- `AgentServiceBinding` collection maps `(api_key_id, user_service_id)` to an override `user_api_key_id`
- Proxy handler checks `agent_service_bindings` before credential injection; falls back to service default if no binding exists
- `PerAgentRateLimiter` in `mw/rate_limit.rs` provides per-API-key rate limit buckets (1-second sliding window)
- Proxy responses include `X-NyxID-Agent-Id` header when request was made with an API key

**CLI:**
- `--profile` flag on `AuthArgs`, `LoginArgs`, `BaseUrlArgs`, and all node commands (env: `NYXID_PROFILE`)
- Profile-aware token storage: `~/.nyxid/profiles/{name}/` (default profile uses `~/.nyxid/`)
- Profile name validation: 1-64 chars, alphanumeric + hyphens + underscores only
- Node multi-instance: profile-aware service labels (`dev.nyxid.node.{profile}` / `nyxid-node-{profile}.service`)
- `nyxid api-key create --platform`, `nyxid api-key bind` commands for managing agent identities (consolidated from former `ai-setup agent` subcommands)

**Frontend:**
- API key detail page shows platform selector, rate limit editor, and credential bindings CRUD
- API key table shows platform and bindings count columns

Key files:
- `models/agent_service_binding.rs` -- per-agent credential override model
- `services/agent_binding_service.rs` -- binding CRUD + credential override lookup
- `handlers/agent_bindings.rs` -- REST endpoints under `/api/v1/api-keys/{id}/bindings`
- `services/proxy_service.rs` -- `resolve_agent_credential_override()` for proxy-time binding lookup
- `cli/src/commands/api_key.rs` -- API key management + credential binding commands
- `frontend/src/hooks/use-agent-bindings.ts` -- TanStack Query hooks for bindings CRUD
- `frontend/src/schemas/agent-bindings.ts` -- Zod schemas for bindings and rate limits

### 10. Catalog Metadata and Endpoint Discovery

Rich metadata on `DownstreamService` for AI agent discovery (issue #148). Agents can discover service docs, repos, capabilities, and API endpoints from the catalog without guessing.

**Model fields** (on `DownstreamService`):
- `homepage_url`, `repository_url`, `issues_url` -- validated URLs for docs/source/issues
- `capabilities` -- `ServiceCapabilities` struct with boolean flags: `supports_proxy_read`, `supports_proxy_write`, `supports_proxy_binary_upload`, `supports_direct_downstream_auth`, `supports_authoring_via_nyx`, `supports_websocket`, `supports_streaming`
- `auth_notes`, `known_limitations` -- freeform text (max 4096 chars)
- `required_permissions` -- array of permission strings (max 100 entries, 256 chars each)

**API:**
- `GET /api/v1/catalog?include_all=true` -- includes system services without auth (default filters to connectable services only)
- `GET /api/v1/catalog/{slug}/endpoints` -- fetches and parses OpenAPI spec via hardened `api_docs_service::fetch_spec_json` (DNS pinning, 5MB limit, 60s cache), returns structured endpoint list
- Admin `POST/PUT /services` accepts all metadata fields with URL validation and length limits

**CLI:**
- `nyxid catalog list --all` -- includes system services
- `nyxid catalog show <slug>` -- displays links, capabilities, auth notes, limitations, permissions
- `nyxid catalog endpoints <slug>` -- lists parsed OpenAPI endpoints in table format

**Frontend:**
- Service edit page: "Service Metadata" section with URL inputs, auth notes, limitations, permissions, and capability toggle switches
- Service detail page: "Service Metadata" section with links, notes, permissions, and capability badges

**Legacy migration:** `migrate_legacy_api_spec_url()` runs at startup to rename `api_spec_url` -> `openapi_spec_url` and remove duplicates. Update handler includes `$unset: { api_spec_url: "" }` as belt-and-suspenders.

## File Structure

```
cli/src/
|-- main.rs              # CLI entry point
|-- cli.rs               # Clap subcommand definitions (25 top-level commands, incl. channel-bot)
|-- commands/            # Command implementations (one file per command group, incl. ai_setup.rs with agent subcommands, channel_bot.rs with bot+route CRUD)
|-- api_client.rs        # HTTP client for NyxID API calls
|-- auth.rs              # Token storage and retrieval (file-based session)
|-- output.rs            # Table/JSON output formatting

node-agent/src/
|-- main.rs              # Deprecation wrapper (prints warning, delegates to nyxid node subcommand)
|-- cli.rs               # Clap subcommand definitions (deprecated, use nyxid node instead)
|-- config.rs            # TOML config (server url, node id, encrypted auth token, signing secret, credentials)
|-- ws_client.rs         # WebSocket connection loop, exponential backoff reconnection, graceful shutdown
|-- proxy_executor.rs    # HTTP request execution, credential injection, SSE streaming detection
|-- credential_store.rs  # In-memory decrypted credential store (header or query_param injection)
|-- daemon.rs            # Background service lifecycle (launchd on macOS, systemd on Linux)
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
|-- models/              # MongoDB document structs (40 models, 38 collections, incl. agent_service_binding, node, node_service_binding, mcp_session, openclaw_channel_mapping, user_endpoint, user_api_key, user_service, channel_bot, channel_conversation, channel_event_log, channel_message)
|-- services/            # Business logic (52 services, incl. agent_binding_service, node_service, node_routing_service, node_ws_manager, node_metrics_service, openclaw_channel_service, unified_key_service, catalog_service, user_endpoint_service, user_api_key_service, user_service_service, action_description, channel_bot_service, channel_event_service, channel_routing_service, channel_relay_service, channel_platform, event_dedup_cache, channel_adapters/{telegram,discord,lark,openclaw})
|-- handlers/            # HTTP handlers (51 handler modules, incl. agent_bindings, node_admin, admin_nodes, node_ws, developer_apps, ssh_exec, llms_txt, openclaw_channel, keys, catalog, user_endpoints, user_api_keys_external, user_services_handler, channel_bots, channel_conversations, channel_events, channel_webhooks, channel_relay)
|-- crypto/              # JWT, AES, password hashing, token generation, KeyProvider trait, KMS providers, JWKS
|-- errors/              # AppError enum, ErrorResponse, AppResult
|-- mw/                  # Middleware: auth, rate_limit, security_headers

frontend/src/
|-- pages/               # Route pages (43 pages, incl. nodes, node-detail, admin-nodes, service-detail, providers, ai-setup, keys, key-detail, channel-bots, channel-bot-detail, channel-conversation-detail)
|-- components/          # UI components (auth/, dashboard/, layout/, shared/, ui/; incl. add-key-dialog for unified key creation)
|-- hooks/               # TanStack Query hooks (20 hooks, incl. use-agent-bindings, use-nodes, use-admin-nodes, use-providers, use-developer-apps, use-keys, use-channel-bots, use-channel-conversations, use-channel-messages)
|-- schemas/             # Zod validation schemas (10 schema files + tests, incl. agent-bindings.ts, nodes.ts, channels.ts)
|-- stores/              # Zustand stores (auth-store)
|-- lib/                 # API client, constants, utils
|-- types/               # TypeScript type definitions (7 files, incl. AdminNodeInfo, NodeMetricsInfo, approvals, keys)
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
- `/api-keys` -- CRUD + rotate. `ApiKey` model has scope fields: `allowed_service_ids`, `allowed_node_ids`, `allow_all_services`, `allow_all_nodes` (absorbed from deleted AgentGroup model). Also has agent isolation fields: `rate_limit_per_second`, `rate_limit_burst`, `platform`
- `/api-keys/{id}/bindings` -- agent credential binding CRUD (list, create, delete). Maps an API key (agent) to a per-service credential override via `AgentServiceBinding`
- `/services` -- CRUD + OIDC credentials + endpoints + requirements
- `/sessions` -- list sessions
- `/connections` -- connect/disconnect services
- `/providers` -- CRUD + OAuth/device-code/API-key flows + token management + per-user credentials
- `/admin` -- user management, audit log, OAuth clients, service accounts
- `/proxy/{service_id}/{path}` -- authenticated proxy (UUID-based); supports HTTP and WebSocket passthrough
- `/proxy/s/{slug}/{path}` -- authenticated proxy (slug-based); supports HTTP and WebSocket passthrough
- `/proxy/services` -- service discovery (paginated list of proxyable services)
- `/llm` -- LLM gateway (provider proxy, OpenAI-compatible gateway, status)
- `/delegation/refresh` -- refresh delegated access tokens
- `/notifications` -- notification settings CRUD, Telegram link/disconnect, device token management (register/list/remove)
- `/approvals` -- approval request history, grants, decide, status polling, per-service approval configs (with `approval_mode`: `per_request` default or `grant` opt-in)
- `/webhooks/telegram` -- Telegram webhook (unauthenticated, secret-verified)
- `/nodes` -- node management (register-token, list, get, delete, rotate-token, bindings CRUD + priority update)
- `/nodes/ws` -- WebSocket upgrade for node agent connections (auth via WS protocol, not middleware)
- `/admin/nodes` -- admin node management (list all, get, disconnect, delete -- no ownership check)
- `/integrations/openclaw/channel` -- OpenClaw channel webhook (unauthenticated, HMAC-verified)
- `/integrations/openclaw/mappings` -- OpenClaw channel-to-user mapping CRUD (authenticated)
- `/keys` -- unified key management: auto-provisions UserEndpoint + UserApiKey + UserService from catalog or custom input (CRUD + OAuth flows)
- `/endpoints` -- user-managed target URLs (list, update, delete)
- `/api-keys/external` -- user's external API keys / credentials (list, update, delete)
- `/user-services` -- user's proxy routing config (list, update, delete)
- `/catalog` -- read-only service catalog for users (list templates, get template by slug, `?include_all=true` for full discovery including system services). Supports `/{slug}/endpoints` for OpenAPI endpoint discovery via hardened spec fetch.
- `/channel-bots` -- channel bot registration CRUD
- `/channel-conversations` -- conversation-to-agent routing (CRUD). Maps platform conversations to agent API keys.
- `/channel-relay/reply` -- agent async reply to a platform conversation. **Only async replies are supported** — sync 200+body replies from agent callbacks were removed per ADR-013 / NyxID#221. Agents must return 202 to the callback and post replies here.
- `/channel-relay/messages/{conversation_id}` -- message history for agents
- `/channel-relay/resolve-sender` -- resolve platform sender to NyxID user
- `/channel-events/{conversation_id}` -- HTTP Event Gateway ingress (NyxID#221, ADR-013). Accepts device event envelopes `{event_id, source, type, timestamp, payload, metadata}`, converts to `CallbackPayload` with `platform="device"`, and forwards through the channel relay pipeline. Per-channel rate limited (default 100/s), idempotent via in-memory LRU dedup (5min TTL), metadata-only logging to `channel_event_logs` (no payload persistence).
- `/webhooks/channel/{telegram,discord,lark,feishu}/{bot_id}` -- platform webhook receivers
- `/ssh/{service_id}/certificate` -- issue short-lived SSH user certificate (POST)
- `/ssh/{service_id}` -- SSH-over-WebSocket tunnel (GET, upgrade)
- `/ssh/{service_id}/terminal` -- SSH web terminal (GET, upgrade)
- `/ssh/{service_id}/exec` -- remote command execution (POST)

- `/admin/service-accounts` -- service account CRUD, secret rotation, token revocation, provider management (connect via API key/OAuth redirect/device-code, list, disconnect providers on behalf of SAs)

- `/oauth/token` -- also supports `grant_type=client_credentials` (service accounts), `grant_type=urn:ietf:params:oauth:grant-type:token-exchange` (RFC 8693 delegated access and social token exchange via `subject_token_type=id_token` for native mobile Google/GitHub login)

Top-level: `/health`, `/.well-known/openid-configuration`, `/oauth/*`, `/mcp`, `/llms.txt`, `/llms-full.txt`

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
WS_PASSTHROUGH_MAX_CONNECTIONS=200     # Maximum concurrent WebSocket passthrough connections (default: 200)

# HTTP Event Gateway (NyxID#221, ADR-013 pure passthrough)
CHANNEL_EVENT_RATE_LIMIT_PER_SECOND=100  # Per-channel event rate limit (default: 100)
CHANNEL_EVENT_RATE_LIMIT_BURST=200       # Per-channel burst capacity (default: 200)
CHANNEL_EVENT_DEDUP_CAPACITY=32768       # LRU dedup cache size (default: 32768; sized to honor 5-min window at 100 events/s)
CHANNEL_EVENT_DEDUP_TTL_SECS=300         # Dedup entry TTL (default: 300 = 5 min)

# Registration gate (issue #179)
INVITE_CODE_REQUIRED=true              # Gate new-user registration behind invite codes (default: true). Set to false for public launch.

# Optional
GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET
GITHUB_CLIENT_ID / GITHUB_CLIENT_SECRET
SMTP_HOST / SMTP_PORT / SMTP_USERNAME / SMTP_PASSWORD / SMTP_FROM_ADDRESS
```

## Available Commands

```bash
# CLI (from project root)
source "$HOME/.cargo/env" 2>/dev/null  # Ensure cargo is available
cargo build -p nyxid-cli                # Build CLI binary
cargo install --path cli                # Install CLI as `nyxid`
nyxid --help                            # Verify installation

# Backend (from project root)
cargo build                             # Build backend (local provider only)
cargo build --features aws-kms          # Build with AWS KMS support
cargo build --features gcp-kms          # Build with GCP Cloud KMS support
cargo build --features aws-kms,gcp-kms  # Build with both KMS providers
cargo test                              # Run backend tests
cargo test --all-features               # Run all tests including KMS provider tests
cargo run                               # Start backend (port 3001)

# Node Agent (from project root, via nyxid CLI)
cargo build -p nyxid-cli                # Build CLI binary (includes node subcommand)
cargo test -p nyxid-cli                 # Run CLI tests (includes node agent tests)
nyxid node register --token nyx_nreg_... --url ws://localhost:3001/api/v1/nodes/ws
nyxid node start                        # Start node agent (foreground)
nyxid node agent-status                 # Show local config status
nyxid node credentials list             # List configured credentials
nyxid node openclaw connect --url http://localhost:18789  # Connect OpenClaw (use --credential-env for non-interactive)
nyxid node openclaw status              # Show OpenClaw connection status
nyxid node openclaw disconnect          # Remove OpenClaw credentials

# Node daemon lifecycle (background service, supports --profile for multi-instance)
nyxid node daemon install               # Install as system service (launchd/systemd)
nyxid node daemon start                 # Start background service
nyxid node daemon stop                  # Stop background service
nyxid node daemon restart               # Restart background service
nyxid node daemon status                # Check if installed and running
nyxid node daemon logs --follow         # Tail daemon logs
nyxid node daemon uninstall             # Remove system service

# Node agent via Docker (alternative to native daemon)
nyxid node docker build                                # Build node agent image
nyxid node docker start [--profile <name>]             # Start container (mounts config volume)
nyxid node docker stop [--profile <name>]              # Stop container
nyxid node docker status [--profile <name>]            # Check container status
nyxid node docker logs [--profile <name>]              # Tail container logs

# Agent isolation (api-key subcommands)
nyxid api-key create --name "coding-agent" --platform claude-code
nyxid api-key list                      # List API keys
nyxid api-key show <ID_OR_NAME>         # Show key details + bindings
nyxid api-key bind <ID_OR_NAME> --service <SLUG> --credential <LABEL>  # Credential override
nyxid api-key rotate <ID_OR_NAME>       # Rotate API key
nyxid api-key delete <ID_OR_NAME>       # Delete API key

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

## Design System

Always read DESIGN.md before making any visual or UI decisions.
All font choices, colors, spacing, and aesthetic direction are defined there.
Do not deviate without explicit user approval.
In QA mode, flag any code that doesn't match DESIGN.md.

## Git Workflow

- Conventional commits: `feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`
- Never commit to main directly
- PRs require review
- All tests must pass before merge
