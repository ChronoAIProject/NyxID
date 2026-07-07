## Project Overview

NyxID is an Auth/SSO platform (similar to Supabase Auth) with a Rust backend, React frontend, and CLI tools: user authentication, OAuth/OIDC, MFA, credential brokering, admin management, and MCP proxy. The `nyxid` CLI covers all user-facing operations (services, keys, catalog, nodes, approvals, SSH, MCP, notifications) and includes `nyxid node` for managing on-premise credential nodes.

**Tech Stack:**
- **Backend:** Rust, Axum 0.8, MongoDB 8.0 (`mongodb` 3.5, `bson` 2.15)
- **Frontend:** React 19, TypeScript, Vite 7, TanStack Router + Query, Tailwind CSS 4, Zod 4, Zustand
- **Mobile:** React Native 0.79, Expo 53, TypeScript (iOS + Android approval app)
- **SDK:** TypeScript OAuth 2.0 client (`@nyxids/oauth-core`, `@nyxids/oauth-react`)
- **Dev tools:** Docker Compose (MongoDB + Mailpit), RSA keys for JWT signing

Deep-dive docs live in `docs/` -- notably ENV.md, ORACLE_RELAY.md, AGENT_ISOLATION.md, CHANNEL_BOT_RELAY.md, CHANNEL_EVENT_GATEWAY.md, NODE_PROXY_ARCHITECTURE.md, OPENCLAW_INTEGRATION.md, API_DISCOVERY.md, SSH_NODE_KEY_AUTH.md. Read the relevant doc before working in that subsystem.

## Critical Rules

### 1. MongoDB Model Conventions

- NEVER use `#[serde(skip_serializing)]` on model fields -- prevents `insert_one(&struct)` from storing them
- ALWAYS use `#[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]` on `DateTime<Utc>` fields; for `Option<DateTime<Utc>>` use the `bson_datetime::optional` helper (`models/bson_datetime.rs`)
- IDs are UUID v4 stored as strings in MongoDB `_id`; each model has a `COLLECTION_NAME` constant

### 2. Layer Architecture

Strict separation: `handlers/` -> `services/` -> `models/`
- **models/** -- plain serde structs, `COLLECTION_NAME`, no business logic
- **services/** -- business logic; takes `&mongodb::Database` and `&str` for IDs
- **handlers/** -- HTTP layer; converts `AuthUser.user_id` (Uuid) to string for services; dedicated response structs (never serialize model structs to API responses)
- **crypto/jwt.rs** takes `&Uuid` (kept for signing); **token_service** parses `&str` to `Uuid` internally
- **unified_key_service** -- orchestration layer for the streamlined services architecture; auto-provisions UserEndpoint + UserApiKey + UserService from catalog or custom input in one operation

### 3. Error Handling

`AppError` enum (`errors/mod.rs`, `thiserror`); handlers return `AppResult<T> = Result<T, AppError>`. Variants map to HTTP status codes and numeric error codes; `backend/src/errors/mod.rs` is the authoritative list. Internal/database errors never leak details to clients.

**Reserved numeric code blocks** (single source of truth -- do not duplicate these tables elsewhere):
- 1011-1019 SSH node-key/auth-mode: 1011 `SshNodeKeyMissing`, 1012 `SshHostKeyMismatch`, 1013 `SshNodeExecChannelClosed`, 1014 `SshPrincipalAmbiguous`, 1015 `SshAuthModeUnsupportedForOperation`
- 8000-8005 node/proxy: 8000 `NodeNotFound`, 8001 `NodeOffline`, 8002 `NodeProxyTimeout`, 8003 `NodeRegistrationFailed`, 8004 `NodeCredentialMissing`, 8005 `WsProxyDownstream`
- 8006-8011 pending-credential protocol: 8006 `PendingCredentialDecryptFailed`, 8007 `PendingCredentialVersionUnsupported`, 8008 `PendingCredentialCiphertextTooLarge`, 8009 `PendingCredentialPubkeyAwaiting`, 8010 `PendingCredentialNodeOffline`, 8011 `PendingCredentialQueueFull`
- 9500-9599 device-code binding: 9500 `DeviceCodeNotFound`, 9501 `DeviceCodeExpired`, 9502 `DevicePollSignatureInvalid`, 9503 `DeviceUserCodeInvalid`, 9504 `DeviceCodePending`, 9505 `DeviceCodeAlreadyDelivered`, 9506 `DeviceCodeRateLimited`, 9507 `DeviceCodeLocked`, 9508 `DeviceCodeSlowDown`
- 11000-11099 oracle relay: 11000 `OraclePoolNotFound`, 11001 `OraclePoolSlugTaken`, 11002 `OraclePoolInactive`, 11003 `OracleWorkerTokenInvalid`, 11004 `OracleQueueFull`, 11005 `OracleQuotaExceeded`, 11006 `OracleTaskNotFound`, 11007 `OracleSessionNotFound`, 11008 `OracleSessionClosed`, 11009 `OraclePayloadTooLarge`, 11010 `OracleExtractDisabled`
- 11100 `AnonymousIncompatibleService` (HTTP 400)
- 11200-11207 auth-device login: 11200 `AuthDeviceCodeNotFound`, 11201 `AuthDeviceCodeExpired`, 11202 `AuthDeviceCodePending`, 11203 `AuthDeviceCodeSlowDown`, 11204 `AuthDeviceCodeDenied`, 11205 `AuthDeviceCodeAlreadyDelivered`, 11206 `AuthDeviceCodeRateLimited`, 11207 `AuthDeviceUserCodeInvalid`

### 4. Frontend Patterns

- Zod schemas in `schemas/` (one per domain); React Hook Form + `@hookform/resolvers`
- TanStack Query hooks in `hooks/` (one per domain: `use-auth.ts`, `use-services.ts`, ...); auth state in Zustand (`stores/auth-store.ts`)
- UI components via Radix UI + shadcn/ui pattern (`components/ui/`); no `console.log` in production code

### 5. Security

- No hardcoded secrets -- env vars for all sensitive data; input validation on all endpoints; PKCE for OAuth flows
- AES-256 envelope encryption with pluggable async `KeyProvider` trait (`crypto/aes.rs`, `crypto/key_provider.rs`); AWS KMS (`crypto/aws_kms_provider.rs`, feature `aws-kms`) and GCP Cloud KMS (`crypto/gcp_kms_provider.rs`, feature `gcp-kms`); fallback provider for zero-downtime migration between encryption backends
- All key material in `Zeroizing` wrappers; all Debug impls redact secrets and key identifiers; `MAX_WRAPPED_DEK_SIZE = 1024` enforced on encrypt and decrypt paths
- Middleware: rate limiting (`mw/rate_limit.rs`), security headers (`mw/security_headers.rs`), JWT auth (`mw/auth.rs`)
- Audit logs are tamper-evident for new rows via HMAC-SHA256 hash chaining (`services/audit_chain_service.rs`): new `AuditLog` rows must set `seq`, `prev_hash`, `entry_hash` through the audit service append path; legacy rows without `seq` count as pre-chain, not backfilled. `AUDIT_CHAIN_HMAC_KEY` is an optional 64-hex override; otherwise the key is domain-derived from `ENCRYPTION_KEY` or the JWT private key. v1 does not detect tail truncation without external head anchoring.
- Anonymous catalog endpoints may be enabled only when the service has `identity_propagation_mode = "none"`, `forward_access_token = false`, and `inject_delegation_token = false` (disabled draft rules may be stored on any service); violating admin writes return `AppError::AnonymousIncompatibleService` (400, code 11100). Public execution still force-strips identity propagation, access-token forwarding, delegation-token injection, and downstream auth defaults as defense in depth.

### 5a. Vendor URN Namespace

NyxID-vendored URN types live under `urn:nyxid:params:oauth:<category>:<name>` (the `params:oauth` infix mirrors `urn:ietf:params:oauth:*` so generic vendor-extension parsers recognize the shape). Registered:

- `urn:nyxid:params:oauth:token-type:binding-id` -- RFC 8693 subject_token_type identifying an `OauthBrokerBinding` handle; used at `/oauth/token` with `grant_type=urn:ietf:params:oauth:grant-type:token-exchange`.

Add new entries here when introducing additional vendored URN types.

### 6. Node Proxy Conventions

- `NodeWsManager`: in-memory connection pool shared via `Arc` in `AppState`; `DashMap` for lock-free concurrent access
- `Node.user_id` is the polymorphic owner field (person or org user, matching `UserService`/`UserApiKey`). Do not add a separate `org_id` to node-related models; ACL via `org_service::resolve_owner_access(actor, node.user_id)`.
- Registration tokens carry the chosen owner at mint time; admin role is verified at issuance, not redemption -- a revoked admin can still complete registration within the token TTL (default 1h, `NODE_REGISTRATION_TOKEN_TTL_SECS`). When removing org admins, also delete their pending registration tokens.
- Node auth tokens (`nyx_nauth_...`) and registration tokens (`nyx_nreg_...`) are 32 random bytes; only SHA-256 hashes stored. HMAC signing secrets are generated at registration: SHA-256 hash on server, encrypted locally on the node agent.
- WebSocket handler (`handlers/node_ws.rs`) authenticates in the first message, not via HTTP middleware
- `node_routing_service::resolve_node_route` runs before credential resolution in `execute_proxy()`; returns `NodeRoute` with `fallback_node_ids` for multi-node failover
- Streaming proxy: `proxy_response_start` / chunk frames / `proxy_response_end`; preferred chunk transport is a WS binary frame with a 36-byte request_id prefix, with legacy `proxy_response_chunk` JSON fallback for older servers
- Node metrics (`node_metrics_service`) recorded fire-and-forget after each proxy request; stored as embedded `NodeMetrics` document on the Node model. Node-routed audit events include `"routed_via": "node"` and `"node_id"`.
- `NodeStatus` is an enum (`Online`/`Offline`/`Draining`) -- not a bare string. WS writer channels are bounded (capacity 256); `try_send` treats full buffers as node offline (H4).
- WS auth-frame injection rules live on `DownstreamService.ws_frame_injections` and `UserService.ws_frame_injections`; additive, separate from HTTP `auth_method` injection. `WsFrameDirection` is the trigger direction: a `downstream` rule matches frames from the service and injects the configured response back toward it. Direct and node-routed WS paths emit metadata-only `ws_frame_auth_injected` audit events; never log injected payloads or credentials.
- SSH services use `ssh_auth_mode` (`cert` | `node_key` | `proxy_only`); legacy `certificate_auth_enabled` true/false maps to `cert`/`proxy_only`. Node-key credentials live only in the node-local encrypted store, keyed by `(service_slug, principal)`, with configured `host_key_sha256` enforced by the russh client. Cert-mode `ssh exec` and browser terminal also run through russh on the node agent (backend-issued ephemeral private key + OpenSSH user certificate) and verify target host keys against a node-local TOFU store `ssh_cert_host_keys.toml` in the node config dir; pins are enforced on every session, and a changed key returns `SshHostKeyMismatch` (1012) + `ssh_host_key_mismatch` audit. Manage pins with `nyxid node ssh cert-host-key list|pin|forget` (`pin` pre-seeds/replaces a SHA256 fingerprint; `forget` is the legitimate rotation recovery path; the store is live-reloaded, so no daemon restart). `ssh proxy` is unsupported for `node_key` -- use `ssh exec` or the browser terminal.
- Admin node endpoints (`handlers/admin_nodes.rs`) require admin role and have no ownership check
- `nyxid node daemon` manages background service lifecycle (`cli/src/node/daemon.rs`): launchd LaunchAgent on macOS / systemd user unit on Linux. All node commands support `--profile` for multi-instance: service labels `dev.nyxid.node.{profile}` (macOS) / `nyxid-node-{profile}.service` (Linux), config at `~/.nyxid-node/profiles/{name}/`.

### 7. OpenClaw Integration

OpenClaw is a self-hosted AI gateway integrated at three levels (details: `docs/OPENCLAW_INTEGRATION.md`):

1. **Provider**: seeded as an `api_key` provider with `requires_gateway_url: true`; users supply gateway URL + bearer token. `UserProviderToken.gateway_url` stores the per-user instance URL; `proxy_service::resolve_gateway_url_override()` overrides the service's default `base_url` at proxy time.
2. **Node proxy**: `nyxid node openclaw` stores credentials locally, registers the provider connection, and creates the node service binding in one step. Flow: NyxID -> node agent (WS) -> local OpenClaw; the node agent injects the bearer token from its credential store.
3. **Channels**: `openclaw_channel_service` handles inbound webhooks from OpenClaw channels (WhatsApp, Telegram, Discord, ...); `openclaw_channel_mappings` maps channel users to NyxID users, each with its own per-user webhook secret (SHA-256 hash stored, raw secret returned once). No server-level env var needed.

Key files: `services/openclaw_channel_service.rs`, `handlers/openclaw_channel.rs`, `models/user_provider_token.rs` (`gateway_url`), `models/provider_config.rs` (`requires_gateway_url`).

### 8. Streamlined Services Architecture

Services/connections/providers were unified into 3 user-managed collections plus one orchestration layer; old collections are kept for backward compatibility during migration.

- Collections: `user_endpoints` (target URLs, custom or from catalog), `user_api_keys` (external credentials: API keys, OAuth tokens, bearer tokens), `user_services` (proxy routing config: endpoint + key + auth method + optional node + identity propagation + custom User-Agent override)
- `unified_key_service` auto-provisions all 3 records from a single `POST /api/v1/keys` request, using catalog defaults or user-provided values
- Proxy resolution checks `UserService` first (by slug + user_id), then falls back to the legacy `DownstreamService` + `UserProviderToken` path for unmigrated users
- OAuth resource indicators: RFC 8707 resource URIs for user services are deterministic and not stored. Canonical URI: `{BASE_URL}/api/v1/proxy/s/{slug}` (`UserService.slug`); catalog and user-services responses expose `resource_uri`. OAuth `resource` parameters resolve to active `UserService` rows through the same personal/org ACL model used by proxy resolution, and JWTs keep the fixed NyxID audience while carrying granted resource URIs in `resources` plus `allowed_service_ids` for enforcement.
- Proxy User-Agent: client UA is forwarded as-is by default; `UserService.custom_user_agent` / `DownstreamService.custom_user_agent` overrides it in all four proxy paths (direct HTTP, node HTTP, direct WS, node WS). Use for downstreams whose WAFs block SDK UA strings (e.g. `OpenAI/Python`).
- `ApiKey` scope fields (absorbed from the deleted `AgentGroup` model): `allowed_service_ids`, `allowed_node_ids`, `allow_all_services`, `allow_all_nodes`; enforced at proxy time via `key_service`
- Frontend: unified "AI Services" page at `/keys` with 2 tabs (External Services; NyxID API Keys with scope). Services/Connections/Providers removed from the normal user sidebar (admin-only); old `/api-keys` page deleted.
- Legacy models kept for migration: DownstreamService (now the read-only catalog), UserServiceConnection, UserProviderToken, UserProviderCredentials, NodeServiceBinding (node routing absorbed into `UserService.node_id`)

Key files: `services/unified_key_service.rs`, `services/catalog_service.rs` (`list_catalog_all`), `handlers/keys.rs`, `handlers/catalog.rs`, `models/user_{endpoint,api_key,service}.rs`.

### 9. Agent Isolation

Per-agent credential binding, rate limiting, and audit attribution: each agent (Claude Code, Codex, OpenClaw, ...) uses its own scoped API key (`nyxid_ag_` prefix). Details: `docs/AGENT_ISOLATION.md`.

- `AuthUser` carries `api_key_id`, `api_key_name`, `rate_limit_per_second`, `rate_limit_burst` when auth is via API key; `ApiKey` has `rate_limit_per_second`, `rate_limit_burst`, `platform`; `AuditLog` has `api_key_id`, `api_key_name` for attribution
- `AgentServiceBinding` maps `(api_key_id, user_service_id)` to an override `user_api_key_id`; the proxy handler checks bindings before credential injection and falls back to the service default
- `PerAgentRateLimiter` (`mw/rate_limit.rs`): per-API-key buckets, 1-second sliding window. Proxy responses include `X-NyxID-Agent-Id` when the request used an API key.
- CLI: `--profile` flag on `AuthArgs`, `LoginArgs`, `BaseUrlArgs`, and all node commands (env `NYXID_PROFILE`); token storage `~/.nyxid/profiles/{name}/` (default profile uses `~/.nyxid/`); profile names 1-64 chars, alphanumeric + hyphens + underscores
- `nyxid api-key create --platform` and `nyxid api-key bind` manage agent identities (consolidated from former `ai-setup agent` subcommands)
- `--org` flags accept UUID, slug, or display name, resolved in that order: UUID locally, slug via `GET /orgs/{slug}`, display name via one `GET /orgs` fetch (errors with candidate rows when ambiguous). Org users have an auto-generated `slug` (visible in `nyxid org list`). `nyxid service add-ssh` also accepts `--org` for org-owned SSH services.
- Frontend: API key detail page has platform selector, rate limit editor, and bindings CRUD; the key table shows platform and bindings count

Key files: `models/agent_service_binding.rs`, `services/agent_binding_service.rs`, `handlers/agent_bindings.rs`, `services/proxy_service.rs` (`resolve_agent_credential_override`), `cli/src/commands/api_key.rs`, `frontend/src/hooks/use-agent-bindings.ts`, `frontend/src/schemas/agent-bindings.ts`.

### 10. Catalog Metadata and Endpoint Discovery

Rich metadata on `DownstreamService` so agents can discover service docs, repos, capabilities, and API endpoints without guessing (issue #148; details: `docs/API_DISCOVERY.md`).

- Fields: `homepage_url` / `repository_url` / `issues_url` (validated URLs); `capabilities` (`ServiceCapabilities` boolean flags: `supports_proxy_read`, `supports_proxy_write`, `supports_proxy_binary_upload`, `supports_direct_downstream_auth`, `supports_authoring_via_nyx`, `supports_websocket`, `supports_streaming`); `auth_notes` / `known_limitations` (max 4096 chars); `required_permissions` (max 100 entries, 256 chars each)
- API: `GET /api/v1/catalog?include_all=true` includes system services without auth (default filters to connectable); `GET /api/v1/catalog/{slug}/endpoints` fetches + parses the OpenAPI spec via hardened `api_docs_service::fetch_spec_json` (DNS pinning, 5MB limit, 60s cache); admin `POST/PUT /services` accepts all metadata fields with URL validation and length limits
- CLI: `nyxid catalog list --all`, `nyxid catalog show <slug>`, `nyxid catalog endpoints <slug>`. Frontend: "Service Metadata" sections on the service edit and detail pages.
- Legacy: `migrate_legacy_api_spec_url()` runs at startup to rename `api_spec_url` -> `openapi_spec_url` and remove duplicates; the update handler also includes `$unset: { api_spec_url: "" }`.

### 11. Oracle Relay (browser LLM pools)

Generic async task relay: any NyxID user/agent calls a browser-driven LLM (ChatGPT Pro, etc.) via `/api/v1/oracle`. A logged-in ChatGPT tab running a worker client is a **worker**; a **pool** is the capacity unit that owns the worker token. Consumers submit prompts and poll for answers; browser/LLM specifics live entirely in the worker, so the backend stays a neutral relay. Full design: `docs/ORACLE_RELAY.md`.

- **Models** (all UUID-string `_id`, `COLLECTION_NAME`): `oracle_pool` (polymorphic `user_id` owner like Node/UserService; `visibility` = private/org/platform; `worker_token_hash`; quotas), `oracle_task` (prompt/response bodies + lease + status + TTL `expires_at`), `oracle_session` (multi-turn `conv_...`), `oracle_worker` (tab presence, id = `{pool_id}:{label}`)
- **Queue** (`oracle_task_service`): MongoDB-backed, no in-memory state -- any instance serves any request. FIFO claim via `find_one_and_update` sorted by `created_at`; `task_timeout_secs` lease (default 4h) refreshed by `ack` heartbeats; expired leases requeue to the FIFO front (preserved `created_at`); idempotent re-claim survives mid-task tab reload; `client_ref` gives submitter-scoped submit idempotency (partial unique index); empty or `ERROR:`-prefixed results -> `failed`
- **Worker token** `nyx_owk_...` (32 random bytes, SHA-256 stored, shown once, rotatable). Worker endpoints (`/api/v1/oracle/worker/{task,ack,result,pin-conv-url}`) authenticate by Bearer token inside the handler, mounted outside JWT middleware. `ack` returning `{status:"cancelled"}` is the cancellation back-channel.
- **ACL**: `oracle_pool_service::ensure_can_submit` gates by visibility via `org_service::resolve_owner_access`; `ensure_can_manage` (owner/org-admin) gates pool mutation + token rotation. Unauthorized task/session reads return a not-found-shaped error (don't leak existence).
- **Privacy**: prompt/response bodies live only on the task doc (TTL `ORACLE_TASK_RETENTION_DAYS`, default 30); audit + tracing are metadata-only (ids/sizes/outcomes), never the prompt or answer -- same discipline as WS frame injection
- **Attach/scrape**: `OracleTask.kind` (`prompt`|`scrape`) + `OracleSession.origin` (`nyxid`|`imported`), both serde-defaulted. `POST /pools/{slug}/attach {chatgpt_url}` mints an imported session + a `kind=scrape` control task; the worker scrapes the full transcript and `POST /oracle/worker/transcript` imports it as completed (user,assistant) turns (atomic claim-guard, staggered `created_at` for order). `list_session_tasks` hides scrape tasks. An existing ChatGPT conversation thus becomes a first-class session (`oracle session`, `oracle ask --conversation`).
- **CLI**: `nyxid oracle ask` (submit + poll; `--no-wait`, `--pdf`, `--new-conversation`/`--conversation`, `--client-ref`), `oracle attach <pool> <url>`, `oracle result/cancel/status/sessions/session/close-session`, `oracle pool create/list/show/update/rotate-token`
- **Workers** (two interchangeable clients, same worker API): (a) `integrations/oracle/nyxid_oracle.user.js` -- Tampermonkey userscript (zero local process; GM config `nyxid_base_url`/`nyxid_worker_token`/`nyxid_worker_label`, `?nyx=N` -> `tab_N`; project pinning server-driven via pool `chatgpt_project_url`); (b) `integrations/oracle/cdp-worker/` -- Node + `playwright-core` daemon that `connectOverCDP`s to the user's real logged-in Chrome (launched with `--remote-debugging-port`), reusing the same extractors via `page.evaluate`; lower friction, no backend change

Key files: `models/oracle_{pool,task,session,worker}.rs`, `services/oracle_{pool,task,session}_service.rs`, `handlers/oracle_{pools,tasks,worker}.rs`, `cli/src/commands/oracle.rs`, `integrations/oracle/nyxid_oracle.user.js`.

### 12. Auth Device-Code Login

First-party RFC 8628-style login for headless CLI environments: `nyxid login --device` displays a short `user_code`; an already logged-in human approves it in the web UI; the waiting CLI receives normal first-party access + refresh tokens without a password in the terminal.

- **Storage**: `auth_device_codes` collection -- HMACs of both the opaque `device_code` and display `user_code` (raw codes and token plaintext never persisted), `pending`/`approved`/`denied`/`expired`/`delivered` status state machine, sanitized client context, encrypted delivery tokens. The CLI polls with an opaque `nyx_adc_`-prefixed `device_code` secret (not a worker-pool flow).
- **Routes**: `POST /auth/device/request` and `POST /auth/device/poll` are public (no auth). `POST /auth/device/approve` is human-only: JWT session user; API keys, service accounts, and delegated tokens are rejected before handler execution. `POST /auth/device/preview` is public and returns non-sensitive anti-phishing context (`client_label`, `client_user_agent`, timestamps, status) for a `user_code` without changing state -- all fields are device-supplied at `/request` time so there is nothing to leak; the verification page only calls it after login (GitHub-style flow), but it stays public as defense-in-depth simplification and for future anonymous-preview surfaces. Rate-limited per IP.
- **Atomic delivery**: poll claims `approved` -> `delivered` via `find_one_and_update(...).return_document(Before)`; the pre-update document carries the encrypted delivery tokens for exactly one poller; later pollers get `AuthDeviceCodeAlreadyDelivered` (11205).
- **Rate limiters**: `auth_device_{request,poll,approve,approve_per_user,preview}_limiter`
- **CLI output**: prints `user_code` plus the bare `verification_uri` -- never `verification_uri_complete`, which would put the code in the URL and defeat manual-entry anti-phishing. On stdin+stderr TTYs: prompts `Open in your browser? [Y/n]` and opens via `crate::browser::open_browser`, falling through with a "paste it manually" hint on failure. Non-TTY (CI/piped) skips the prompt and polls immediately.
- **CLI dispatch**: `nyxid login --device` forces device-code login; plain `nyxid login` auto-falls back to it when browser launch is unavailable unless `NYXID_LOGIN_NO_DEVICE_FALLBACK=1`.
- **Verification page** (`/login/device`): auth-gated at entry (redirect to `/login?return_to=/login/device`). Code input starts empty; the `?user_code=` query param is stripped by the router's `validateSearch` and ignored. Two explicit clicks: Continue calls `/preview` (anti-phishing block renders), then Approve calls `/approve`. No API call fires on typing, focus, mount, or reconnect; both buttons are throttled to >= 750 ms between clicks and disabled while pending (input disabled too).
- **Out of scope for v1**: mobile approval deep-link to the approval app (follow-up issue if needed).

## File Structure

```
cli/src/
|-- main.rs              # CLI entry point
|-- cli.rs               # Clap subcommand definitions
|-- commands/            # Command implementations (one file per command group)
|-- api_client.rs        # HTTP client for NyxID API calls
|-- auth.rs              # Token storage and retrieval (file-based session)
|-- output.rs            # Table/JSON output formatting

backend/src/
|-- config.rs            # AppConfig from env vars
|-- db.rs                # MongoDB connection + ensure_indexes()
|-- routes.rs            # All route definitions
|-- main.rs              # Server startup
|-- models/              # MongoDB document structs, one per collection
|-- services/            # Business logic (incl. channel_adapters/{telegram,discord,lark,openclaw})
|-- handlers/            # HTTP handlers
|-- crypto/              # JWT, AES, password hashing, token generation, device-code Ed25519 verification, KeyProvider trait, KMS providers, JWKS
|-- errors/              # AppError enum, ErrorResponse, AppResult
|-- mw/                  # Middleware: auth, rate_limit, security_headers

frontend/src/
|-- pages/               # Route pages
|-- components/          # UI components (auth/, dashboard/, layout/, shared/, ui/)
|-- hooks/               # TanStack Query hooks (one per domain)
|-- schemas/             # Zod validation schemas with vitest specs
|-- stores/              # Zustand stores (auth-store)
|-- lib/                 # API client, constants, utils
|-- types/               # TypeScript type definitions
|-- router.tsx           # TanStack Router config

mobile/src/              # React Native + Expo app (Expo 53, RN 0.79): app/ (shell, navigator,
                         # deep linking nyxid://challenge/{id}), features/ (auth, challenges,
                         # approvals, account, legal), components/, lib/ (API client, SecureStore
                         # session, push registration), theme/

sdk/                     # OAuth SDK monorepo (@nyxids/* npm namespace): oauth-core (PKCE OAuth 2.0
                         # client, NyxIDClient), oauth-react (React context + useNyxID() hook),
                         # demo-react (private demo app)
```

## Key API Routes

All API routes under `/api/v1`:
- `/auth` -- register, login, logout, refresh, verify-email, forgot/reset-password
- `/auth/device/{request,poll,approve,preview}` -- auth device-code login (see Critical Rule 12 for auth posture per route)
- `/auth/mfa` -- setup, confirm, verify (login), disable (nested under `/auth` in `routes.rs`; `setup` is idempotent against unverified factors per NyxID#506)
- `/users` -- get/update current user
- `/api-keys` -- CRUD + rotate; `ApiKey` scope + agent isolation fields per Critical Rules 8-9
- `/api-keys/{id}/bindings` -- agent credential binding CRUD (`AgentServiceBinding`)
- `/services` -- CRUD + OIDC credentials + endpoints + requirements
- `/sessions` -- list sessions
- `/connections` -- connect/disconnect services
- `/providers` -- CRUD + OAuth/device-code/API-key flows + token management + per-user credentials
- `/admin` -- user management, audit log, OAuth clients, service accounts
- `/proxy/{service_id}/{path}` and `/proxy/s/{slug}/{path}` -- authenticated proxy (UUID- and slug-based); HTTP + WebSocket passthrough
- `/proxy/services` -- service discovery (paginated list of proxyable services)
- `/llm` -- LLM gateway (provider proxy, OpenAI-compatible gateway, status)
- `/delegation/refresh` -- refresh delegated access tokens
- `/notifications` -- notification settings CRUD, Telegram link/disconnect, device token management
- `/approvals` -- approval history, grants, decide, status polling, per-service approval configs (`approval_mode`: `per_request` default or `grant` opt-in)
- `/webhooks/telegram` -- Telegram webhook (unauthenticated, secret-verified)
- `/devices/code/request` -- unauthenticated device-code binding start for headless devices; returns `device_code`, `user_code`, verification URLs
- `/devices/code/poll` -- unauthenticated but Ed25519-signed device poll; returns pending user-code rotations or one-time credentials after approval
- `/devices/code/approve` -- authenticated approval (web + CLI); creates the scoped API key, node row, and refresh token. `DeviceCodeApprove.default_services` is an opt-in list of user-service UUIDs/slugs granted proxy access at approval; omit for an empty allowlist.
- `/devices/onboard` -- authenticated server-side QR provisioning for no-WiFi headless devices; creates the scoped API key + device node stub, embeds WiFi + raw one-time credentials in a `nyxprov://` QR payload, stores only the refresh-token hash, never persists the WiFi password
- `/nodes` -- node management (register-token, list, get, delete, rotate-token, bindings CRUD + priority update)
- `/nodes/ws` -- WebSocket upgrade for node agents (auth via WS protocol, not middleware)
- `/admin/nodes` -- admin node management (list all, get, disconnect, delete -- no ownership check)
- `/integrations/openclaw/channel` -- OpenClaw channel webhook (unauthenticated, HMAC-verified); `/integrations/openclaw/mappings` -- mapping CRUD (authenticated)
- `/keys` -- unified key management: auto-provisions UserEndpoint + UserApiKey + UserService from catalog or custom input (CRUD + OAuth flows)
- `/endpoints`, `/api-keys/external`, `/user-services` -- list/update/delete for the three user collections
- `/catalog` -- read-only service catalog (`?include_all=true` for full discovery; `/{slug}/endpoints` for OpenAPI endpoint discovery via hardened spec fetch)
- `/oracle` -- browser LLM relay (Critical Rule 11). Consumer side (JWT or `nyxid_ag_` key): `/pools` CRUD + `/pools/{slug}/rotate-token`, `/pools/{slug}/tasks`, `/pools/{slug}/status`, `/tasks/{id}` + `/cancel`, `/sessions` + `/sessions/{conv_id}` + `/close`. Worker side `/oracle/worker/*` authenticates by pool worker token inside the handler, outside JWT middleware. 16 MiB body cap (PDF attach / multi-MB answers).
- `/channel-bots` -- channel bot registration CRUD + PATCH for platform verification material
- `/channel-conversations` -- conversation-to-agent routing CRUD (platform conversations -> agent API keys)
- `/channel-relay/reply` -- agent async reply to a platform conversation. Only async replies are supported (sync 200+body callback replies removed per ADR-013 / NyxID#221): agents return 202 to the callback and post here. Dual auth: (a) agent API key (`Bearer nyxid_ag_...`, scoped by `conversation.agent_api_key_id`); (b) per-callback `reply_token` from the inbound payload -- a single-use RS256 JWT, `aud="channel-relay/reply"`, bound to one `inbound_message_id` + `conversation_id` + `api_key_id` + `platform`, TTL `JWT_RELAY_REPLY_TTL_SECS`, use tracked in MongoDB `reply_token_uses`. Lets downstream runtimes (e.g. Aevatar) reply without persisting agent keys. Details: `docs/CHANNEL_BOT_RELAY.md`.
- `/channel-relay/reply/update` -- edit a previously-sent reply by the upstream `platform_message_id`; same dual auth. A reply token may edit only if its JTI already exists in `reply_token_uses` (proves it was used to send first). V1: Lark/Feishu only; other platforms return `edit_unsupported`.
- `/channel-relay/messages/{conversation_id}` -- message history for agents; `/channel-relay/resolve-sender` -- resolve platform sender to NyxID user
- `/channel-events/{conversation_id}` -- HTTP Event Gateway ingress (NyxID#221, ADR-013). Accepts device event envelopes `{event_id, source, type, timestamp, payload, metadata}`, converts to `CallbackPayload` with `platform="device"`, forwards through the channel relay pipeline. Per-channel rate limited (default 100/s), idempotent via in-memory LRU dedup (5 min TTL), metadata-only logging to `channel_event_logs` (no payload persistence).
- `/webhooks/channel/{telegram,discord,lark,feishu}/{bot_id}` -- platform webhook receivers
- `/ssh/{service_id}/certificate` (POST, issue short-lived SSH user cert), `/ssh/{service_id}` (WS tunnel), `/ssh/{service_id}/terminal` (WS web terminal), `/ssh/{service_id}/exec` (POST, remote command execution)
- `/admin/service-accounts` -- service account CRUD, secret rotation, token revocation, provider management on behalf of SAs
- `/oauth/token` -- also supports `grant_type=client_credentials` (service accounts) and RFC 8693 token exchange (`grant_type=urn:ietf:params:oauth:grant-type:token-exchange`) for delegated access and social token exchange (`subject_token_type=id_token` for native mobile Google/GitHub login)

Top-level: `/health`, `/.well-known/openid-configuration`, `/oauth/*`, `/mcp`, `/llms.txt`, `/llms-full.txt`

## Channel Bot Notes

Lark / Feishu developer-console fields serve different purposes -- do not conflate:
- **App ID + App Secret**: authenticate outbound NyxID calls to Lark/Feishu APIs (tenant access tokens, sending replies)
- **Verification Token**: required for inbound webhook verification; compared against `header.token` on v2 events or top-level `token` on v1 / `url_verification` payloads
- **Encrypt Key**: optional; when configured, NyxID verifies `X-Lark-Signature`, decrypts the `encrypt` payload, then validates the Verification Token on the decrypted JSON

A bot stuck in `pending_webhook`: patch it with its Verification Token (and optional Encrypt Key) via `PATCH /api/v1/channel-bots/{id}` or `nyxid channel-bot update <ID> --verification-token ... [--encrypt-key ...] [--app-id ...] [--app-secret ...]`; the next verified inbound auto-promotes it to `active`.

## Environment Variables

Full semantics for the long-form entries: `docs/ENV.md`.

```bash
# Required
DATABASE_URL=mongodb://...          # MongoDB connection string
ENCRYPTION_KEY=                     # 64 hex chars (AES-256); required for local, optional for KMS (enables fallback)
ENCRYPTION_KEY_PREVIOUS=            # Optional previous key for zero-downtime rotation
KEY_PROVIDER=local                  # "local" (default), "aws-kms", "gcp-kms" (feature-gated)

# KMS (optional; --features aws-kms / gcp-kms; *_PREVIOUS for rotation)
AWS_KMS_KEY_ARN= / AWS_KMS_KEY_ARN_PREVIOUS=
GCP_KMS_KEY_NAME= / GCP_KMS_KEY_NAME_PREVIOUS=

# Defaults provided
PORT=3001
BASE_URL=http://localhost:3001
FRONTEND_URL=http://localhost:3000
JWT_PRIVATE_KEY_PATH=keys/private.pem
JWT_PUBLIC_KEY_PATH=keys/public.pem
JWT_ISSUER=nyxid
JWT_ACCESS_TTL_SECS=900             # 15 min
JWT_REFRESH_TTL_SECS=604800         # 7 days
JWT_RELAY_REPLY_TTL_SECS=1800       # Per-callback reply token TTL
JWT_RELAY_CALLBACK_TTL_SECS=300     # Callback authentication JWT TTL
JWT_RELAY_ACCESS_TTL_SECS=300       # X-NyxID-User-Token relay access token TTL. Relay tokens are
                                    # proxy/LLM-only (rejected elsewhere by reject_relay_tokens),
                                    # inherit the agent key's service/node allowlist, and are
                                    # invalidated when that key is revoked (ensure_relay_agent_key_active).
SA_TOKEN_TTL_SECS=3600              # Service account tokens
ENVIRONMENT=development
RATE_LIMIT_PER_SECOND=10
RATE_LIMIT_BURST=30
TRUSTED_PROXY_IPS=                  # Reverse-proxy IPs whose X-Forwarded-For/X-Real-IP are trusted for
                                    # per-IP rate-limit keying. Empty = trust only the TCP peer. Only list
                                    # proxies configured to overwrite client-supplied forwarded headers.
MTLS_CLIENT_CERT_HEADER=            # Header carrying a URL-encoded PEM client cert from a trusted
                                    # mTLS-terminating proxy (RFC 8705 cert-bound broker tokens).
                                    # Unset = disabled. The proxy must strip this header from external requests.
CLI_PAIRING_HMAC_KEY=               # Optional 64 hex; keys CliPairing.code_hash against DB-snapshot
                                    # brute force. Unset = derived from ENCRYPTION_KEY or the JWT key
                                    # (stable per-worker, multi-instance safe).
AUDIT_CHAIN_HMAC_KEY=               # Optional 64 hex; keys audit-log HMAC chaining; same derivation fallback

CHANNEL_RELAY_CALLBACK_TIMEOUT_SECS=30
CHANNEL_RELAY_MAX_BOTS_PER_USER=5
CHANNEL_RELAY_MESSAGE_TTL_DAYS=30
CHANNEL_RELAY_EDIT_RATE_LIMIT_PER_SECOND=10   # Per-platform-message edit rate limit
CHANNEL_RELAY_EDIT_RATE_LIMIT_BURST=20

# Telegram / Approval System (optional)
TELEGRAM_BOT_TOKEN=                 # From @BotFather
TELEGRAM_WEBHOOK_SECRET=            # Random string for webhook verification
TELEGRAM_WEBHOOK_URL=               # e.g. https://auth.nyxid.dev/api/v1/webhooks/telegram
TELEGRAM_BOT_USERNAME=              # Without @
APPROVAL_EXPIRY_INTERVAL_SECS=5     # Interval between expiry sweeps

# OAuth token refresh (optional)
OAUTH_REFRESH_SWEEP_INTERVAL_SECS=600  # Proactive refresh sweep for expiring multi-connection OAuth
                                       # tokens; 0 disables (lazy proxy-time refresh still applies).
                                       # Does NOT extend refresh-token lifetime (a Google app in
                                       # "Testing" still expires refresh tokens after 7 days).
OAUTH_REFRESH_SWEEP_WINDOW_SECS=900    # Look-ahead window; keep larger than the proxy-time 5-min buffer

# Mobile Push (optional)
FCM_SERVICE_ACCOUNT_PATH=           # Firebase service account JSON
APNS_KEY_PATH= / APNS_KEY_ID= / APNS_TEAM_ID=
APNS_TOPIC=                         # iOS bundle ID (e.g. dev.nyxid.app)
APNS_SANDBOX=true                   # Default true in dev

# Credential Nodes (optional, defaults shown)
NODE_HEARTBEAT_INTERVAL_SECS=30
NODE_HEARTBEAT_TIMEOUT_SECS=90      # Mark offline after N secs without heartbeat
NODE_PROXY_TIMEOUT_SECS=30
NODE_REGISTRATION_TOKEN_TTL_SECS=3600
NODE_MAX_PER_USER=10
NODE_MAX_WS_CONNECTIONS=100
NODE_MAX_STREAM_DURATION_SECS=300
NODE_HMAC_SIGNING_ENABLED=true
WS_PASSTHROUGH_MAX_CONNECTIONS=200

# HTTP Event Gateway (NyxID#221, ADR-013)
CHANNEL_EVENT_RATE_LIMIT_PER_SECOND=100
CHANNEL_EVENT_RATE_LIMIT_BURST=200
CHANNEL_EVENT_DEDUP_CAPACITY=32768  # Sized to honor the 5-min window at 100 events/s
CHANNEL_EVENT_DEDUP_TTL_SECS=300

INVITE_CODE_REQUIRED=true           # Gate registration behind invite codes (issue #179); false for public launch
AUTO_VERIFY_EMAIL=false             # Dev only: skip email verification on registration

# Optional
GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET
GITHUB_CLIENT_ID / GITHUB_CLIENT_SECRET
SMTP_HOST / SMTP_PORT / SMTP_USERNAME / SMTP_PASSWORD / SMTP_FROM_ADDRESS
```

## Available Commands

```bash
# CLI (from project root; end users install prebuilt release binaries, cargo install is for dev)
source "$HOME/.cargo/env" 2>/dev/null   # Ensure cargo is available
cargo build -p nyxid-cli                # Build CLI binary (includes node subcommand)
cargo test -p nyxid-cli                 # CLI tests (includes node agent tests)
cargo install --path cli                # Install as `nyxid`
nyxid login --device                    # Headless browser-assisted login;
                                        # NYXID_LOGIN_NO_DEVICE_FALLBACK=1 disables auto-fallback from plain `nyxid login`

# Backend (from project root)
cargo build [--features aws-kms,gcp-kms]   # Feature-gated KMS providers
cargo test [--all-features]
cargo run                               # Start backend (port 3001)

# Node agent
nyxid node register --token nyx_nreg_... --url ws://localhost:3001/api/v1/nodes/ws
nyxid node start | agent-status | credentials list
nyxid node openclaw connect --url http://localhost:18789   # --credential-env for non-interactive
nyxid node openclaw status | disconnect
nyxid node daemon install|start|stop|restart|status|logs --follow|uninstall   # launchd/systemd; supports --profile
nyxid node docker build|start|stop|status|logs [--profile <name>]             # Docker alternative to native daemon

# Agent isolation
nyxid api-key create --name "coding-agent" --platform claude-code
nyxid api-key list | show <ID_OR_NAME> | rotate <ID_OR_NAME> | delete <ID_OR_NAME>
nyxid api-key bind <ID_OR_NAME> --service <SLUG> --credential <LABEL>   # Credential override

# Device-code binding
nyxid device approve XXXX-XXXX-XXXX [--org <ID|SLUG|NAME>] [--label <LABEL>] [--service <SLUG_OR_UUID>]...
nyxid device onboard --label "Kitchen Camera" --ssid "MyNetwork" --password-env WIFI_PASSWORD [--org ...] [--service ...]...
nyxid device factory-key [--count N] [--out FILE] [--ndjson]

# Channel bots
nyxid channel-bot register --platform telegram --label support --token-env TELEGRAM_BOT_TOKEN
nyxid channel-bot register --platform lark --label support --token-env LARK_BOT_TOKEN \
  --app-id cli_xxx --app-secret-env LARK_APP_SECRET --verification-token vtoken_xxx
nyxid channel-bot update <BOT_ID> --verification-token ... [--encrypt-key ...]
  # env alternatives: NYXID_LARK_VERIFICATION_TOKEN / NYXID_LARK_ENCRYPT_KEY

# Frontend (from frontend/)
npm run dev | build | test | test:watch | lint   # dev = port 3000; build = type-check + prod build

# Mobile (from mobile/): npm run start | ios | android
# SDK (from sdk/): npm run build | clean
# Docker (from project root):
docker compose up -d                    # MongoDB (27018) + Mailpit (8025)
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


<!-- consensus-rnd:foundational-invariants:start version=1 sha256=f5c24b0c3515993a7b86c4ed78ce7386add665f8c8b84cc7275aedebd6c3e6af -->
## 共识研发不动点（由 consensus-rnd 管理）

- FI-001 AI 产物默认不可信；进入主线前必须经过独立检查，至少包含共识、review 或自动验证中的适用组合。
- FI-002 Host 事实必须由 host 配置或 host 规则注入；通用 skill / engine 不硬编码具体项目、组织、路径、分支或人员事实；skill-private runtime directories such as `.refactor-loop/` must not become host production configuration or ledger SSOT.
- FI-003 稳定核心保持小而可审计；高频变化留在 host 规则、prompt、脚本或扩展层，不下沉为核心不变量。
- FI-004 跨进程、跨 turn 或跨节点的事实必须有权威记录；进程内记忆、cache、临时变量不能冒充事实源。
- FI-005 边界优先于便利；职责、层级、协议和状态所有权必须清楚，禁止用中间层快捷方式绕过主链路。
- FI-006 变更必须可验证且基于 evidence；失败、缺口和越界承诺要显式暴露，禁止用静默假设或禁用测试换取通过。
- FI-007 删除优先；废弃路径直接移除，除非 host 规则明确要求迁移期兼容。
<!-- consensus-rnd:foundational-invariants:end -->
