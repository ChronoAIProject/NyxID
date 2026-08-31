## Project Overview

NyxID is an Auth/SSO platform (similar to Supabase Auth) with a Rust backend, React frontend, and CLI tools: user authentication, OAuth/OIDC, MFA, credential brokering, admin management, and MCP proxy. The `nyxid` CLI covers all user-facing operations (services, keys, catalog, nodes, approvals, SSH, MCP, notifications) and includes `nyxid node` for managing on-premise credential nodes.

**Tech Stack:**
- **Backend:** Rust, Axum 0.8, MongoDB 8.0 (`mongodb` 3.5, `bson` 2.15)
- **Frontend:** React 19, TypeScript, Vite 7, TanStack Router + Query, Tailwind CSS 4, Zod 4, Zustand
- **Mobile:** React Native 0.83, Expo 55, TypeScript (iOS + Android approval app)
- **SDK:** TypeScript OAuth 2.0 client (`@nyxids/oauth-core`, `@nyxids/oauth-react`)
- **Dev tools:** Docker Compose (MongoDB + Mailpit), RSA keys for JWT signing

Deep-dive docs live in `docs/` -- notably chat/README.md, ENV.md, ORACLE_RELAY.md, AGENT_ISOLATION.md, CHANNEL_BOT_RELAY.md, CHANNEL_EVENT_GATEWAY.md, NODE_PROXY_ARCHITECTURE.md, OPENCLAW_INTEGRATION.md, API_DISCOVERY.md, SSH_NODE_KEY_AUTH.md. Read the relevant doc before working in that subsystem.

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
- 8012 `ClientDisconnected` (HTTP 499, nginx's "Client Closed Request"): the caller hung up before the response could be written and upstream work was cancelled. Never delivered to anyone — it exists so cancelled work is not counted as a server fault in telemetry and audit. Do not map it to 5xx.
- 9500-9599 device-code binding: 9500 `DeviceCodeNotFound`, 9501 `DeviceCodeExpired`, 9502 `DevicePollSignatureInvalid`, 9503 `DeviceUserCodeInvalid`, 9504 `DeviceCodePending`, 9505 `DeviceCodeAlreadyDelivered`, 9506 `DeviceCodeRateLimited`, 9507 `DeviceCodeLocked`, 9508 `DeviceCodeSlowDown`
- 11000-11099 oracle relay: 11000 `OraclePoolNotFound`, 11001 `OraclePoolSlugTaken`, 11002 `OraclePoolInactive`, 11003 `OracleWorkerTokenInvalid`, 11004 `OracleQueueFull`, 11005 `OracleQuotaExceeded`, 11006 `OracleTaskNotFound`, 11007 `OracleSessionNotFound`, 11008 `OracleSessionClosed`, 11009 `OraclePayloadTooLarge`, 11010 `OracleExtractDisabled`
- 11100 `AnonymousIncompatibleService` (HTTP 400)
- 11200-11207 auth-device login: 11200 `AuthDeviceCodeNotFound`, 11201 `AuthDeviceCodeExpired`, 11202 `AuthDeviceCodePending`, 11203 `AuthDeviceCodeSlowDown`, 11204 `AuthDeviceCodeDenied`, 11205 `AuthDeviceCodeAlreadyDelivered`, 11206 `AuthDeviceCodeRateLimited`, 11207 `AuthDeviceUserCodeInvalid`
- 11300-11305 connect links: 11300 `ConnectLinkNotFound`, 11301 `ConnectLinkExpired`, 11302 `ConnectLinkAlreadyCompleted`, 11303 `ConnectLinkCancelled`, 11304 `ConnectLinkRateLimited`, 11305 `ConnectLinkCompletionInProgress`
- 11500 `GrantCascadeConfirmationRequired` (HTTP 409)
- 11600-11606 triggers: 11600 `TriggerNotFound`, 11601 `TriggerSecretInvalid`, 11602 `TriggerRateLimited`, 11603 `TriggerPayloadTooLarge`, 11604 `TriggerDeliveryUnsupported`, 11605 `TriggerDeliveryFailed`, 11606 `TriggerDeliveryRecordNotFound`
- 11700 `RequestBodyTooLarge` (HTTP 413): a bounded proxy or forwarding ingress exceeded its configured byte limit

### 4. Frontend Patterns

- Zod schemas in `schemas/` (one per domain); React Hook Form + `@hookform/resolvers`
- Forms use `useAppForm` (`components/ui/form.tsx`), never raw `useForm` (lint-enforced via `no-restricted-imports`): its `setValue` defaults to `shouldDirty/shouldTouch/shouldValidate: true` so non-text controls wired via `watch` + `setValue` (Radix Switch/Select/Checkbox, custom editors) correctly enable dirty-gated submit buttons; programmatic writes (prefill, normalization in effects) opt out per call with `{ shouldDirty: false, shouldTouch: false }`
- TanStack Query hooks in `hooks/` (one per domain: `use-auth.ts`, `use-services.ts`, ...); auth state in Zustand (`stores/auth-store.ts`)
- UI components via Radix UI + shadcn/ui pattern (`components/ui/`); no `console.log` in production code

### 5. Security

- No hardcoded secrets -- env vars for all sensitive data; input validation on all endpoints; PKCE for OAuth flows
- AES-256 envelope encryption with pluggable async `KeyProvider` trait (`crypto/aes.rs`, `crypto/key_provider.rs`); AWS KMS (`crypto/aws_kms_provider.rs`, feature `aws-kms`) and GCP Cloud KMS (`crypto/gcp_kms_provider.rs`, feature `gcp-kms`); fallback provider for zero-downtime migration between encryption backends
- All key material in `Zeroizing` wrappers; all Debug impls redact secrets and key identifiers; `MAX_WRAPPED_DEK_SIZE = 1024` enforced on encrypt and decrypt paths
- Middleware: rate limiting (`mw/rate_limit.rs`), security headers (`mw/security_headers.rs`), JWT auth (`mw/auth.rs`)
- Audit logs are tamper-evident for new rows via HMAC-SHA256 hash chaining (`services/audit_chain_service.rs`): new `AuditLog` rows must set `seq`, `prev_hash`, `entry_hash` through the audit service append path; legacy rows without `seq` count as pre-chain, not backfilled. `AUDIT_CHAIN_HMAC_KEY` is an optional 64-hex override; otherwise the key is domain-derived from `ENCRYPTION_KEY` or the JWT private key. v1 does not detect tail truncation without external head anchoring.
- Billing transactions are tamper-evident via the append-only `billing_ledger` collection (`services/billing/ledger.rs`): charged usage settlements (first-apply only, including the wallet-lock crash bridge), provider-confirmed top-up checkouts, paid wallet credits (deduped by Lago invoice id), grant lifecycle movements, and purchased-credit expiry append hash-chained entries with the same `seq`/`prev_hash`/`entry_hash` construction, keyed by `BILLING_LEDGER_HMAC_KEY` or the `billing-ledger` domain derivation. Legacy usage/top-up append paths remain best-effort and never roll back billing. Grant issuance is unspendable until journaled, grant consumption retains its settlement lock until journal confirmation, grant issuance/terminal markers retry during reconciliation, and purchased-credit expiry retains its wallet operation until every ledger entry is durable. Verify via `GET /api/v1/admin/billing-ledger/verify`. Tail truncation is detected: the reconcile sweep anchors the ledger head into the audit chain (`billing_ledger_head_anchored`) and the server log, and verify cross-checks the newest anchor (`tail_truncated` break); entries newer than the last anchor (<= one reconcile interval) are the remaining window. Both chains are also re-verified automatically in rolling chunks (`services/chain_verify_service.rs`, `CHAIN_VERIFY_INTERVAL_SECS`), with status persisted to `chain_verify_status`, surfaced on the admin Integrity page (`/admin/integrity`), and escalated via error-level logs.
- Per-service platform prices are authored in NyxID at `ServiceBilling.platform_pricing` and synchronized to stable Lago `platform_svc_{slug}` sum metrics and standard charges on `LAGO_PLAN_CODE`. Plan updates must round-trip the full existing charge array with ids because Lago replaces it. Pending/failed syncs and charge-removal cleanup retry during reconciliation; a service continues using its legacy plan metric until the new price is `synced`, and services without a NyxID price retain legacy Lago-authored pricing.
- Credit benefits live in `credit_grants`, `credit_schedules`, `credit_schedule_periods`, `usage_allowances`, and `usage_allowance_periods`. Funding precedence is actual-quantity allowance units, soonest-expiring grant microcredits, then wallet credits. Only the wallet-funded quantity is emitted to Lago or included in drift comparison. Grant expiry is enforced at reservation time and swept for terminal ledgering. Recurring windows are UTC. `one_time` applies only to allowances. Both credit schedules and allowances support `daily`, `weekly`, and `monthly`. Grant issuance journals at most 50 one-shot recipients inline. Deferred rows are unspendable until reconcile confirms their ledger entries. Admin/operator endpoints are under `/api/v1/admin/credits`. User reads are `/api/v1/billing/{grants,allowances}` and accept an authorized org `owner_id`.
- A scheduled grant is an ordinary `CreditGrant`. Its UUID v5 `_id` is derived from `{schedule_id}:{period_start_ms}:{recipient_user_id}` and is the disbursement identity. Keep `grant-issued:{grant_id}` as the ledger dedupe key. Treat period rows and leases as derived progress, never as money locks. Each period freezes its policy and all-owner signup snapshot when claimed. Catch-up mints only the current window and abandons elapsed incomplete periods. Pausing opens no new period, but an in-flight walk completes. Schedules use the existing billing reconcile interval and add no environment variable.
- Purchased Lago credits expire independently 365 days after settlement. Lago v1.50's wallet-wide `expiration_at` cannot model rolling purchases, so `services/billing/topup_expiry.rs` uses FIFO transaction remainders and Lago `voided_credits`. `BillingWallet.active_topup_expiry` is the crash bridge and `pending_topup_expiry_credits` must be subtracted by every availability expression. The existing billing reconcile interval runs this sweep; there is no separate expiry environment variable.
- Anonymous catalog endpoints may be enabled only when the service has `identity_propagation_mode = "none"`, `forward_access_token = false`, and `inject_delegation_token = false` (disabled draft rules may be stored on any service); violating admin writes return `AppError::AnonymousIncompatibleService` (400, code 11100). Public execution still force-strips identity propagation, access-token forwarding, delegation-token injection, and downstream auth defaults as defense in depth.
- Exact-service approvals bind an execution-authority v2 digest (`services/execution_authority.rs`) over resolved execution inputs, including `token_exchange_config`, `service_category`, and `requires_user_credential`, while retaining the real v1 digest for rolling compatibility. Main-era v1-only rows validate against the live v1 projection; pre-digest rows skip that one gate within their approval expiry. The additive delegated exact-view fields retain the v2 contract: rolling rows keep the pre-additive digest for old replicas and separately bind the full projection for new replicas. Durable `ServiceEndpoint` operations also bind the server-resolved positive generation; instance-spec and legacy unbound rows skip only the generation comparison while every other fence remains. Observe and redeem share one ordered live evaluator (`catalog/exact generation -> policy -> read-only execution authority`); observe reports transient drift without writes, while redeem claims first and durably records terminal revalidation failures before credential materialization. After those gates, redeem materializes credentials and compares execution authority again before any provider effect. `UserApiKey.credential_epoch` versions credential material: bump it on user-initiated credential *replacement* only, never on background/lazy OAuth refresh, and always via a **pipeline** update (`vec![doc! { "$set": ... }]`) -- the bump is an aggregation expression and a classic update document silently stores it as a literal sub-document. See `docs/GRANULAR_APPROVALS_DESIGN.md`.
- Delegated management parity requires the exact `account:read` scope, `GET`, a non-WebSocket request, and a route outside the deny classes in `mw/auth.rs`; the verified `AuthUser` extractor is authoritative. New secret-delivering, execution-shaped, streaming, upgrade, or authentication/provisioning protocol GET routes MUST be added to `delegated_read_denied_path` before mounting.

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
- OAuth resource indicators: RFC 8707 resource URIs for user services are deterministic and not stored. The canonical URI is `{BASE_URL}/api/v1/proxy/s/{slug}` where `{slug}` is `UserService.slug`; catalog and user-services responses expose this as `resource_uri`. OAuth `resource` parameters resolve active `UserService` rows through the same personal/org ACL model used by proxy resolution. Issued JWTs keep the fixed NyxID audience while carrying granted resource URIs in a `resources` claim plus `allowed_service_ids` for enforcement.
- Proxy User-Agent: client UA is forwarded as-is by default; `UserService.custom_user_agent` / `DownstreamService.custom_user_agent` overrides it in all four proxy paths (direct HTTP, node HTTP, direct WS, node WS). Use for downstreams whose WAFs block SDK UA strings (e.g. `OpenAI/Python`).
- `ApiKey` scope fields (absorbed from the deleted `AgentGroup` model): `allowed_service_ids`, `allowed_node_ids`, `allow_all_services`, `allow_all_nodes`; enforced at proxy time via `key_service`
- Frontend: unified "AI Services" page at `/keys` with 2 tabs (External Services; NyxID API Keys with scope). Services/Connections/Providers removed from the normal user sidebar (admin-only); old `/api-keys` page deleted.
- Legacy models kept for migration: DownstreamService (now the read-only catalog), UserServiceConnection, UserProviderToken, UserProviderCredentials, NodeServiceBinding (node routing absorbed into `UserService.node_id`)
- Lifecycle is exactly two actions, named **Disable/Enable** (reversible, sets `UserService.is_active`) and **Delete** (hard-deletes credential + endpoint, leaves an `is_active: false` tombstone). Do not introduce further synonyms; `revoked` is a credential *status*, not a button verb. `GET /keys` is the one listing that returns disabled services — consumers MUST read `is_active` rather than assume every row is usable, and MUST NOT render `status` (the credential's) as the service's state. Every credential-resolving or catalog path keeps the active-only `list_user_services_with_sources`; only `list_keys` uses the `_including_disabled` variant. `/keys/{id_or_slug}` resolves a disabled row by UUID but deliberately not by slug — full rationale and the two known gaps in `docs/AI_SERVICES_ARCHITECTURE.md`.

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
- Hosted overlay specs: hand-curated OpenAPI 3.1 overlays with `x-aevatar-tool` annotations live in `backend/specs/catalog/` and are served publicly at `/api/v1/catalog-specs/{spec_key}/openapi.json` (`services/catalog_spec_registry.rs`; several slugs may share one overlay, e.g. github / lark-feishu pairs). Seeded rows get `openapi_spec_url` from the registry (insert-time + null-guarded backfill; admin-set URLs never overwritten). `services/catalog_spec_sync.rs` runs at startup and additively upserts `ServiceEndpoint` rows from the overlays (matched by name; admin-added endpoints never soft-deleted) so `/api/v1/mcp/config` publishes concrete `service_id` + `endpoint_id` operations for catalog-backed user services (issue #1290 / Aevatar v4 workflow admission).
- MCP catalog precedence: a valid instance-mounted `openapi_spec_url` on a catalog-backed UserService overrides the template's `ServiceEndpoint` rows (per-instance override wins); a broken/empty instance spec falls back to template rows when they exist, else degrades to the generic proxy tool. No instance spec -> template rows; empty + no spec still drops the service from the operation catalog.
- Spec-URL auto-discovery: active HTTP catalog services with an `openapi_spec_url` and zero `ServiceEndpoint` rows get discovery run automatically (startup background sweep + on admin create/update with a spec URL) via the hardened fetch path; private/internal spec URLs still need the manual `discover-endpoints` route, and services with existing rows are never auto-touched.
- Overlay drift guard: `.github/workflows/catalog-spec-drift.yml` (weekly + manual) runs `scripts/check-catalog-spec-drift.py` to verify every overlay operation still exists in the official upstream spec for providers that publish one (OpenAI, X, Discord, ElevenLabs, Twilio); red = update the overlay by hand, never auto-updated.

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

First-party RFC 8628-style login for headless CLI environments: `nyxid login --device` displays a short `user_code`; an already logged-in human approves or rejects it in the web UI or mobile app; the waiting CLI receives normal first-party access + refresh tokens without a password in the terminal.

- **Storage**: `auth_device_codes` collection -- HMACs of both the opaque `device_code` and display `user_code` (raw codes and token plaintext never persisted), `pending`/`approved`/`denied`/`expired`/`delivered` status state machine, sanitized client context, short-lived server-observed `client_ip` plus its HMAC, encrypted delivery tokens. Denials record `denied_at` and `denied_by_user_id`. The CLI polls with an opaque `nyx_adc_`-prefixed `device_code` secret (not a worker-pool flow).
- **Routes**: `POST /auth/device/request`, `/poll`, and `/preview` are public (no auth). `/poll-web` is also public but is a first-party browser-only delivery surface: it shares the `/poll` limiter, atomically claims the same delivery, revokes the approve-minted JWT session, and creates a fresh `nyx_session` cookie session without returning tokens. Preview is non-mutating and returns untrusted `client_label`/`client_user_agent` plus server-observed `client_ip`, Z-suffix timestamps, and status. `POST /auth/device/approve` and `/deny` are human-only: JWT session user; API keys, service accounts, delegated tokens, and relay tokens are rejected before handler execution. Approve and deny race atomically on `status = pending`; deny mints no tokens and the next poll returns 11204. All routes are rate-limited per IP; decisions also share a per-user limit.
- **Atomic delivery**: poll claims `approved` -> `delivered` via `find_one_and_update(...).return_document(Before)`; the pre-update document carries the encrypted delivery tokens for exactly one poller; later pollers get `AuthDeviceCodeAlreadyDelivered` (11205).
- **Rate limiters**: `auth_device_{request,poll,approve,approve_per_user,preview}_limiter` (`/poll-web` shares `poll`)
- **CLI output**: prints `user_code` plus the bare `verification_uri` -- never `verification_uri_complete`, which would put the code in the URL and defeat manual-entry anti-phishing. On stdin+stderr TTYs: prompts `Open in your browser? [Y/n]` and opens via `crate::browser::open_browser`, falling through with a "paste it manually" hint on failure. Non-TTY (CI/piped) skips the prompt and polls immediately.
- **CLI dispatch**: `nyxid login --device` forces device-code login; plain `nyxid login` auto-falls back to it when browser launch is unavailable unless `NYXID_LOGIN_NO_DEVICE_FALLBACK=1`.
- **Verification page** (`/login/device`): auth-gated at entry (redirect to `/login?return_to=/login/device`). Code input starts empty; the `?user_code=` query param is stripped by the router's `validateSearch` and ignored. Two explicit clicks: Continue calls `/preview` (anti-phishing block renders), then Approve or Reject calls `/approve` or `/deny`. No API call fires on typing, focus, mount, or reconnect; actions are throttled to >= 750 ms between clicks and disabled while pending (input disabled too).
- **Mobile approval**: `CameraView` scans only QR codes whose shape is `https://<frontend>/login/device?user_code=...` or `nyxid://login/device?user_code=...`. The app extracts and normalizes only `user_code`; every request uses its configured API base URL. Deep links prefill without making a request. Preview and approve/reject remain separate explicit actions, share the >= 750 ms throttle, and never auto-approve.
- **Web login**: the unauthenticated login page can start the browser delivery flow with an explicit click, showing both the QR payload and manual code; it never starts on mount or auto-regenerates codes.

### 13. Hosted Connect Links

Single-use hosted credential setup for agents and CLI callers. An authenticated creator requests a catalog service link, gives the returned URL to the same account's human user, and polls until the service is provisioned.

- `connect_links` stores UUID-string IDs and only SHA-256 hashes of `nyx_clk_` tokens. Raw tokens appear once in the hosted URL and must never be logged, audited, or persisted in OAuth state.
- Links default to 15 minutes, are clamped to 60-3600 seconds, and retain an observable `expired` terminal state through atomic query-time expiry claims. An OAuth or device flow pinned before expiry gets 30 minutes of finalization grace, then expires if it remains pending.
- App-bound abandoned links are also claimed by the `CONNECT_LINK_EXPIRY_SWEEP_INTERVAL_SECS` background sweep so `connect_link.expired` webhooks do not depend on polling. `0` disables the sweep.
- Preview is public and rate-limited but non-mutating. Completion is human-session-only; API keys, service accounts, delegated tokens, and relay tokens are rejected before the handler. Personal completion requires the exact creating account; org-owned completion uses `org_service::resolve_owner_access` write access. Unauthorized access is not-found-shaped.
- Normal authorization-code access tokens issued to registered developer apps may create, read, and creator-cancel connect links; their JWT `client_id` binds the link to the active app record. Delegated, relay, and service-account tokens remain rejected on that route group, and no other rejection layer is weakened. App callbacks use `oauth_service::validate_client` against registered redirect policy, while sessions and agent API keys retain shape-only HTTP(S) validation. The authenticated app ID/name overrides untrusted request-body attribution.
- Hosted human decline uses the same raw-token and owner checks as completion but only transitions the link to `cancelled`. Every terminal callback is built from the stored URI with `status` and `connect_link_id`; existing query parameters are preserved, reserved parameters are replaced, and raw tokens are never added. Provider callback errors store only a normalized `last_error` code and timestamp while the link remains retryable.
- API-key and OAuth provisioning must reuse `unified_key_service`; completion is atomically serialized and single-use. OAuth state carries only `connect_link_id`, while the browser keeps the raw token in session storage across the redirect.
- MCP callers use `nyx__connect_service` followed by `nyx__wait_for_connection`. A pending link must not activate service tools or emit `tools/list_changed`; activation happens only after completed status is observed.

### 14. Connection Webhooks and Triggers

- Developer-app connection webhooks use a server-generated secret encrypted with `EncryptionKeys`. Secrets are returned only by configure/rotate responses, alongside a non-secret key ID. Delivery signs `X-NyxID-Timestamp + "." + raw_body` with HMAC-SHA256 and sends the signature, timestamp, event type, delivery ID, and key ID headers. Connect-link terminal events use the link as a durable bounded outbox; connection-expiry events remain best effort. Transition paths never depend on delivery success.
- Outbound webhook configuration requires HTTPS and public DNS/IP targets. Apply `webhook_delivery_service::validate_webhook_url` to every new server-fetched webhook URL.
- Trigger inbound secrets use the `nyx_trg_` prefix and are SHA-256 hashed. HMAC verification additionally retains an encrypted copy because verification requires the raw key. Trigger and delivery types are serde-tagged enums; all secret-bearing structs use redacted `Debug` implementations.
- Trigger ingress is public; unknown and disabled triggers are not-found-shaped, then per-trigger rate limiting runs before body reads, HMAC decryption, or verification. Webhook-target envelopes are persisted only in `trigger_deliveries`, encrypted with `EncryptionKeys`, and TTL-expired for durable dedup and authenticated replay; `TRIGGER_DELIVERY_RETENTION_HOURS=0` stores metadata only. Agent and notification payloads are never persisted; their event IDs use fenced, TTL-expiring dedup claims in MongoDB. Agent targets enter through the trusted channel-event service path without broadening the public channel-event auth contract.

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

mobile/src/              # React Native + Expo app (Expo 55, RN 0.83): app/ (shell, navigator,
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
- `/auth/device/{request,poll,poll-web,preview,approve,deny}` -- auth device-code login (see Critical Rule 12 for auth posture per route)
- `/connect-links` -- create, poll, creator cancel, public preview, human decline, and human-only completion for hosted service connections (see Critical Rule 13)
- `/developer/oauth-clients/{client_id}/connection-webhook` -- human-only developer-app lifecycle webhook configure/disable; `/connection-webhook/rotate-secret` returns a new signing secret and key ID once
- `/triggers` -- trigger CRUD; `/{id}/rotate-secret`, `/{id}/rotate-delivery-secret`, `/{id}/deliveries`, and `/{id}/deliveries/{event_id}/redeliver` cover inbound/outbound secret rotation, delivery history, and retained-envelope replay (JWT or agent API key; delegated, relay, and service-account tokens rejected)
- `/webhooks/triggers/{trigger_id}` -- unauthenticated trigger ingress verified by token or raw-body HMAC (see Critical Rule 14)
- `/auth/mfa` -- setup, confirm, verify (login), disable (nested under `/auth` in `routes.rs`; `setup` is idempotent against unverified factors per NyxID#506)
- `/users` -- get/update current user
- `/api-keys` -- CRUD + rotate; `ApiKey` scope + agent isolation fields per Critical Rules 8-9
- `/api-keys/{id}/bindings` -- agent credential binding CRUD (`AgentServiceBinding`)
- `/services` -- CRUD + OIDC credentials + endpoints + requirements
- `/sessions` -- list sessions
- `/connections` -- connect/disconnect services
- `/providers` -- CRUD + OAuth/device-code/API-key flows + token management + per-user credentials
- `/admin` -- user management, audit log, OAuth clients, service accounts
- `/assistant/actions` -- public static assistant action manifest for Aevatar startup discovery
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
JWT_ASSISTANT_FORWARD_TTL_SECS=300  # Legacy tombstone for the retired assistant_forward token.
                                    # Live assistant delegation uses the compile-time 300s
                                    # MCP_DELEGATION_TOKEN_TTL_SECS constant; this env has no effect.
SA_TOKEN_TTL_SECS=3600              # Service account tokens
ENVIRONMENT=development
RATE_LIMIT_PER_SECOND=10
RATE_LIMIT_BURST=30
PLATFORM_SERVICE_RATE_LIMIT_PER_SECOND=0  # Sustained per-user rate for platform-credentialed services; 0 (default) disables
PLATFORM_SERVICE_RATE_LIMIT_BURST=10      # Per-user burst capacity for platform-credentialed services
TRUSTED_PROXY_IPS=                  # Reverse-proxy IPs whose X-Forwarded-For/X-Real-IP are trusted for
                                    # per-IP rate-limit keying. Empty = trust only the TCP peer. Only list
                                    # proxies configured to overwrite client-supplied forwarded headers.
MTLS_CLIENT_CERT_HEADER=            # Header carrying a URL-encoded PEM client cert from a trusted
                                    # mTLS-terminating proxy (RFC 8705 cert-bound broker tokens).
                                    # Unset = disabled. The proxy must strip this header from external requests.
BROKER_REQUIRE_SENDER_CONSTRAINT=false  # Default-off broker hardening: require DPoP/mTLS-pinned
                                        # broker bindings at create and exchange time. Runtime-overridable
                                        # by platform admins via Admin -> OAuth Clients -> Broker Rollout Policy;
                                        # DB override wins over this env default; other replicas refresh
                                        # from MongoDB on a short background interval. See docs/ENV.md.
BROKER_REQUIRE_ADMIN_CAPABILITY=false   # Default-off broker hardening: require platform-admin
                                        # broker_capability_enabled provisioning; DCR scope and
                                        # non-admin developer-app self-grant no longer confer broker mode.
                                        # Runtime-overridable by platform admins; DB override wins over env.
CLI_PAIRING_HMAC_KEY=               # Optional 64 hex; keys CliPairing.code_hash against DB-snapshot
                                    # brute force. Unset = derived from ENCRYPTION_KEY or the JWT key
                                    # (stable per-worker, multi-instance safe).
AUDIT_CHAIN_HMAC_KEY=               # Optional 64 hex; keys audit-log HMAC chaining; same derivation fallback
BILLING_LEDGER_HMAC_KEY=            # Optional 64 hex; keys billing-ledger HMAC chaining; same derivation fallback
CHAIN_VERIFY_INTERVAL_SECS=3600     # Automatic rolling verification sweep for both hash chains; 0 disables

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
CONNECT_LINK_EXPIRY_SWEEP_INTERVAL_SECS=60  # App connect-link expiry webhooks; 0 disables

# OAuth token refresh (optional)
OAUTH_REFRESH_SWEEP_INTERVAL_SECS=600  # Proactive refresh sweep for expiring multi-connection OAuth
                                       # tokens; 0 disables (lazy proxy-time refresh still applies).
                                       # Does NOT extend refresh-token lifetime (a Google app in
                                       # "Testing" still expires refresh tokens after 7 days).
OAUTH_REFRESH_SWEEP_WINDOW_SECS=900    # Look-ahead window; keep larger than the proxy-time 5-min buffer
CONNECTION_EXPIRY_NOTIFICATIONS=true   # Notify once when OAuth dies; transition audits remain enabled

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
PROXY_MAX_BODY_SIZE=104857600       # Authenticated proxy + MCP raw bodies; direct and node-routed
LLM_MAX_BODY_SIZE=10485760          # /api/v1/llm provider + gateway bodies
PUBLIC_PROXY_MAX_BODY_SIZE=1048576  # Anonymous public proxy + public MCP bodies
WS_PASSTHROUGH_MAX_CONNECTIONS=200

# HTTP Event Gateway (NyxID#221, ADR-013)
CHANNEL_EVENT_RATE_LIMIT_PER_SECOND=100
CHANNEL_EVENT_RATE_LIMIT_BURST=200
CHANNEL_EVENT_DEDUP_TTL_SECS=300
TRIGGER_RATE_LIMIT_PER_SECOND=10
TRIGGER_RATE_LIMIT_BURST=20
TRIGGER_PAYLOAD_MAX_BYTES=262144
TRIGGER_DELIVERY_RETENTION_HOURS=72  # 0 keeps metadata only and disables replay

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


<!-- consensus-rnd:foundational-invariants:start version=1 sha256=180aef50d385eb24f73e772f75753fa61a2adde8407539bbacd586878d7e2166 -->
## Consensus R&D Foundational Invariants (managed by consensus-rnd)

- FI-001 AI outputs are untrusted by default; before entering the mainline they must pass independent checks, including the applicable mix of consensus, review, or automated verification.
- FI-002 Host facts must be injected by host configuration or host rules; generic skills / engines must not hardcode project, organization, path, branch, or personnel facts. Skill-private runtime directories such as `.refactor-loop/` must not become host production configuration or ledger SSOT.
- FI-003 Keep the stable core small and auditable; frequently changing behavior belongs in host rules, prompts, scripts, or extension layers, not in core invariants.
- FI-004 Facts that cross processes, turns, or nodes must have an authoritative record; in-process memory, caches, and temporary variables cannot pretend to be sources of truth.
- FI-005 Boundaries take priority over convenience; responsibility, layering, protocols, and state ownership must be clear, and shortcuts through intermediate layers must not bypass the main path.
- FI-006 Changes must be verifiable and evidence-based; failures, gaps, and out-of-bounds commitments must be surfaced explicitly, not hidden behind silent assumptions or disabled tests.
- FI-007 Prefer deletion; remove deprecated paths directly unless host rules explicitly require migration-period compatibility.
<!-- consensus-rnd:foundational-invariants:end -->
