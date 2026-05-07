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

### 5a. Vendor URN Namespace

NyxID-vendored URN types live under `urn:nyxid:params:oauth:<category>:<name>`. The `params:oauth` infix mirrors the IETF style at `urn:ietf:params:oauth:*` so generic OAuth vendor-extension parsers recognize the suffix shape. Currently registered:

- `urn:nyxid:params:oauth:token-type:binding-id` — RFC 8693 subject_token_type identifying an `OauthBrokerBinding` handle. Used at `/oauth/token` with `grant_type=urn:ietf:params:oauth:grant-type:token-exchange`.

Add new entries here when introducing additional vendored URN types.

### 6. Node Proxy Conventions

- `NodeWsManager` is an in-memory connection pool shared via `Arc` in `AppState`; uses `DashMap` for lock-free concurrent access
- `Node.user_id` is the polymorphic owner field, matching `UserService` / `UserApiKey`: it may point to a person user or an org user (`user_type=Org`). Do not add a separate `org_id` to node-related models; use `org_service::resolve_owner_access(actor, node.user_id)` for ACL.
- Registration tokens carry the chosen owner at mint time; admin role is verified at issuance, not at redemption. A revoked admin can still complete a node registration within the token TTL (default 1h, `NODE_REGISTRATION_TOKEN_TTL_SECS`). Operators removing org admins should also delete pending registration tokens for that owner.
- Node auth tokens (`nyx_nauth_...`) and registration tokens (`nyx_nreg_...`) are 32-byte random values; only SHA-256 hashes are stored
- HMAC signing secrets are generated at registration; stored as SHA-256 hashes on server, encrypted locally on the node agent
- WebSocket handler (`handlers/node_ws.rs`) authenticates in the first message, not via HTTP middleware
- Proxy routing check (`node_routing_service::resolve_node_route`) runs before credential resolution in `execute_proxy()`; returns `NodeRoute` with `fallback_node_ids` for multi-node failover
- Streaming proxy uses `proxy_response_start` / chunk frames / `proxy_response_end`; preferred chunk transport is a WebSocket binary frame with a 36-byte request_id prefix, with legacy `proxy_response_chunk` JSON fallback for older servers
- Node metrics (`node_metrics_service`) are recorded asynchronously (fire-and-forget) after each proxy request; stored as embedded `NodeMetrics` document on the Node model
- Node-routed audit events include `"routed_via": "node"` and `"node_id"` in event data
- Error codes 8000-8003 are reserved for node errors (`NodeNotFound`, `NodeOffline`, `NodeProxyTimeout`, `NodeRegistrationFailed`)
- Error codes 1011-1019 are reserved for SSH-specific node-key/auth-mode errors:
  - 1011 `SshNodeKeyMissing`
  - 1012 `SshHostKeyMismatch`
  - 1013 `SshNodeExecChannelClosed`
  - 1014 `SshPrincipalAmbiguous`
  - 1015 `SshAuthModeUnsupportedForOperation`
- `NodeStatus` is an enum (`Online`/`Offline`/`Draining`) -- not a bare string
- WS writer channels are bounded (capacity: 256); `try_send` treats full buffers as node offline (H4)
- WebSocket auth-frame injection rules live on `DownstreamService.ws_frame_injections` and `UserService.ws_frame_injections`; they are additive and separate from HTTP `auth_method` injection. `WsFrameDirection` is the trigger direction, so a `downstream` rule matches frames from the service and injects the configured response back toward that service. Direct and node-routed WS paths emit metadata-only `ws_frame_auth_injected` audit events; never log injected payloads or credentials.
- SSH services use `ssh_auth_mode` (`cert`, `node_key`, `proxy_only`) instead of deriving behavior from `certificate_auth_enabled` alone. Legacy `certificate_auth_enabled=true` maps to `cert`; `false` maps to `proxy_only`. Node-key SSH credentials live only in the node-local encrypted store and are keyed by `(service_slug, principal)`. `ssh proxy` is unsupported for `node_key`; use `ssh exec` or the browser terminal.
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
### 8. Streamlined Services Architecture

The services/connections/providers system was unified into 3 user-managed collections with a single orchestration layer. Old collections are kept for backward compatibility during migration.

**New user collections:**
- `user_endpoints` -- target URLs (custom or auto-provisioned from catalog)
- `user_api_keys` -- external credentials (API keys, OAuth tokens, bearer tokens)
- `user_services` -- proxy routing config (binds endpoint + key + auth method + optional node + identity propagation + custom User-Agent override)

**Orchestration:** `unified_key_service` auto-provisions all 3 records from a single `POST /api/v1/keys` request, using catalog defaults or user-provided values.

**Proxy resolution:** New path checks `UserService` first (by slug + user_id), falls back to old `DownstreamService` + `UserProviderToken` path for unmigrated users.

**Proxy User-Agent:** By default, the client's `User-Agent` header is forwarded as-is (passthrough). When `UserService.custom_user_agent` or `DownstreamService.custom_user_agent` is set, it overrides the client UA on outgoing requests. Applied in all four proxy paths: direct HTTP, node HTTP, direct WS, node WS. Use this for downstreams whose WAFs block SDK-specific UA strings (e.g. `OpenAI/Python`).

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
- Organization `--org` flags accept UUID, slug, or display name. The CLI resolves in that order: UUID returns locally, slug calls `GET /orgs/{slug}`, and display-name lookup fetches `GET /orgs` once and errors with candidate rows when ambiguous. Org users have an auto-generated `slug`, visible in `nyxid org list`.
- `nyxid service add-ssh` also accepts `--org` and creates org-owned SSH services the same way `nyxid service add` creates org-owned HTTP services.

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
- `/channel-bots` -- channel bot registration CRUD + PATCH updates for platform verification material
- `/channel-conversations` -- conversation-to-agent routing (CRUD). Maps platform conversations to agent API keys.
- `/channel-relay/reply` -- agent async reply to a platform conversation. **Only async replies are supported** — sync 200+body replies from agent callbacks were removed per ADR-013 / NyxID#221. Agents must return 202 to the callback and post replies here. Accepts two auth modes: (a) the agent's API key (`Authorization: Bearer nyxid_ag_…`), scoped by `conversation.agent_api_key_id`; or (b) a per-callback reply token (`Authorization: Bearer <reply_token>`) delivered in the inbound callback payload's `reply_token` field. Reply tokens are RS256 JWTs with `aud="channel-relay/reply"`, bound to one `inbound_message_id` + `conversation_id` + `api_key_id` + `platform`, single-use (enforced via MongoDB `reply_token_uses`), and valid for `JWT_RELAY_REPLY_TTL_SECS` (default 30 min). Intended for downstream runtimes (e.g. Aevatar) that want to reply without persisting agent API keys.
- `/channel-relay/reply/update` -- agent edit of a previously-sent platform reply, addressed by the upstream `platform_message_id` returned from `/channel-relay/reply`. Accepts the same dual auth as `/channel-relay/reply`: agent API key or the original per-callback reply token. Reply tokens remain `aud="channel-relay/reply"` and single-use for the initial send; edit requests revalidate the same token and require its JTI to already exist in `reply_token_uses`, which proves the token was previously used to send before it can edit. V1 platform support: Lark / Feishu only; other bot platforms return `edit_unsupported`.
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

## Channel Bot Notes

For Lark / Feishu channel bots, the developer console fields are used for different purposes and must not be conflated:

- **App ID** + **App Secret** authenticate outbound NyxID calls to the Lark / Feishu APIs so NyxID can fetch tenant access tokens and send replies.
- **Verification Token** is required for inbound webhook verification. NyxID compares it against `header.token` on v2 events or top-level `token` on v1 / `url_verification` payloads.
- **Encrypt Key** is optional. When configured in the Lark / Feishu Event Subscriptions console, NyxID verifies `X-Lark-Signature`, decrypts the `encrypt` payload, and then validates the Verification Token on the decrypted JSON.

If an older Lark / Feishu bot is stuck in `pending_webhook`, patch the bot with its Verification Token and optional Encrypt Key, then wait for the next verified inbound to auto-promote it to `active`:

- `PATCH /api/v1/channel-bots/{id}`
- `nyxid channel-bot update <ID> --verification-token ... [--encrypt-key ...] [--app-id ...] [--app-secret ...]`

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
JWT_RELAY_REPLY_TTL_SECS=1800       # 30 minutes (per-callback reply token TTL)
JWT_RELAY_CALLBACK_TTL_SECS=300     # 5 minutes (callback authentication JWT TTL)
SA_TOKEN_TTL_SECS=3600              # 1 hour (service account tokens)
ENVIRONMENT=development
RATE_LIMIT_PER_SECOND=10
RATE_LIMIT_BURST=30
TRUSTED_PROXY_IPS=                     # Comma-separated list of reverse-proxy IPs
                                        # whose `X-Forwarded-For` / `X-Real-IP` may
                                        # be trusted for per-IP rate-limit keying
                                        # (CLI-pairing claim: 5/60s). Empty default
                                        # means "trust only the TCP peer" — safe for
                                        # direct-exposure deployments. Behind nginx /
                                        # ALB / Fly.io, set this to the proxy IPs so
                                        # each user gets their own bucket instead of
                                        # colliding on a single proxy-wide bucket.
                                        # ONLY list proxies you've configured to
                                        # overwrite client-supplied forwarded
                                        # headers. See docs/ENV.md.

MTLS_CLIENT_CERT_HEADER=                # Optional header name carrying a URL-encoded PEM
                                        # client certificate from a trusted mTLS-terminating
                                        # reverse proxy. Leave unset/empty to disable
                                        # RFC 8705 certificate-bound broker access tokens.
                                        # When set, configure the proxy to strip this
                                        # header from external requests before forwarding.

# CLI remote pairing (optional)
CLI_PAIRING_HMAC_KEY=                   # 64 hex chars; keys `CliPairing.code_hash`
                                        # so a DB snapshot cannot brute-force the
                                        # ~2^40 pairing-code space. Leave unset
                                        # unless you need to rotate it independently
                                        # of ENCRYPTION_KEY / the JWT signing key:
                                        # the backend derives from ENCRYPTION_KEY
                                        # when set, otherwise from the JWT private
                                        # key PEM. Both are stable per-worker so
                                        # multi-instance deployments stay in sync
                                        # without extra config. See docs/ENV.md.

CHANNEL_RELAY_CALLBACK_TIMEOUT_SECS=30          # Callback timeout for inbound agent delivery (default: 30)
CHANNEL_RELAY_MAX_BOTS_PER_USER=5               # Maximum bots a user can register (default: 5)
CHANNEL_RELAY_MESSAGE_TTL_DAYS=30               # Channel message TTL in MongoDB (default: 30 days)
CHANNEL_RELAY_EDIT_RATE_LIMIT_PER_SECOND=10     # Per-platform-message edit rate limit (default: 10)
CHANNEL_RELAY_EDIT_RATE_LIMIT_BURST=20          # Per-platform-message edit burst capacity (default: 20)

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

# Dev convenience
AUTO_VERIFY_EMAIL=false                # When true, skip email verification on registration (default: false). Dev only.

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

# Channel bots
nyxid channel-bot register --platform telegram --label support --token-env TELEGRAM_BOT_TOKEN
nyxid channel-bot register --platform lark --label support --token-env LARK_BOT_TOKEN --app-id cli_xxx --app-secret-env LARK_APP_SECRET --verification-token vtoken_xxx
nyxid channel-bot update <BOT_ID> --verification-token vtoken_xxx --encrypt-key key_xxx
NYXID_LARK_VERIFICATION_TOKEN=vtoken_xxx NYXID_LARK_ENCRYPT_KEY=key_xxx nyxid channel-bot update <BOT_ID>

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
