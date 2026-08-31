# Environment Variables

All configuration is loaded from environment variables. A `.env` file is supported via `dotenvy`. Copy `.env.example` as a starting template.

For deployment-specific guidance on these variables, see [DEPLOYMENT.md](DEPLOYMENT.md).

---

## Required

| Variable | Description | Example |
|----------|-------------|---------|
| `DATABASE_URL` | MongoDB connection string | `mongodb://localhost:27017/nyxid` |
| `ENCRYPTION_KEY` | 32-byte hex-encoded AES-256 key (64 hex chars) | Output of `openssl rand -hex 32` |

## Encryption

| Variable | Default | Description |
|----------|---------|-------------|
| `ENCRYPTION_KEY_PREVIOUS` | *(none)* | Previous encryption key for zero-downtime key rotation (64 hex chars). Set this to the old `ENCRYPTION_KEY` value when rotating keys. With envelope encryption, KEK rotation only re-wraps per-record DEK blobs (via `rewrap()`) without re-encrypting data. One previous key supported at a time; finish re-wrapping before rotating again. See [SECURITY.md](SECURITY.md#key-rotation) for the full procedure and `/health` decrypt counters. |
| `KEY_PROVIDER` | `local` | Key provider backend: `local` (default), `aws-kms` (requires `--features aws-kms`), `gcp-kms` (requires `--features gcp-kms`) |

### AWS KMS (optional, requires `--features aws-kms`)

Uses the standard AWS credential chain: environment variables, `~/.aws/credentials`, or IAM role (ECS/EC2/EKS IRSA). `AWS_REGION` or `AWS_DEFAULT_REGION` must also be set.

| Variable | Description |
|----------|-------------|
| `AWS_KMS_KEY_ARN` | Full ARN of AWS KMS key (required when `KEY_PROVIDER=aws-kms`) |
| `AWS_KMS_KEY_ARN_PREVIOUS` | Previous AWS KMS key ARN for rotation |
| `AWS_ACCESS_KEY_ID` | AWS access key (or use IAM role) |
| `AWS_SECRET_ACCESS_KEY` | AWS secret key (or use IAM role) |
| `AWS_REGION` | AWS region (e.g. `us-east-1`) |

### GCP Cloud KMS (optional, requires `--features gcp-kms`)

Uses GCP Application Default Credentials: `GOOGLE_APPLICATION_CREDENTIALS` env var, `gcloud auth application-default login`, or GCE/GKE metadata server. The service account needs the "Cloud KMS CryptoKey Encrypter/Decrypter" role.

| Variable | Description |
|----------|-------------|
| `GCP_KMS_KEY_NAME` | Full GCP KMS key resource name (required when `KEY_PROVIDER=gcp-kms`) |
| `GCP_KMS_KEY_NAME_PREVIOUS` | Previous GCP KMS key name for rotation |
| `GOOGLE_APPLICATION_CREDENTIALS` | Path to service account JSON file |

See [KMS_MIGRATION_GUIDE.md](KMS_MIGRATION_GUIDE.md) and [KMS_OPERATIONS_GUIDE.md](KMS_OPERATIONS_GUIDE.md) for migration and operational procedures.

## Server

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `3001` | HTTP listen port |
| `BASE_URL` | `http://localhost:3001` | Backend base URL (used in JWT `aud`) |
| `FRONTEND_URL` | `http://localhost:3000` | Frontend origin for CORS |
| `ENVIRONMENT` | `development` | `development`, `staging`, `production` |

## Cluster coordination

NyxID uses MongoDB for shared ownership, leases, replay checks, rate windows,
capacity slots, and MCP notifications. The internal HTTP listener forwards only
operations that must reach a process-owned node WebSocket. Do not route this
listener through a public Service, ingress, or load balancer.

| Variable | Default | Description |
|----------|---------|-------------|
| `INSTANCE_NAME` | `POD_NAME`, then `HOSTNAME` | Stable name for one pod or host. A random process generation fences restarts that reuse this name. Kubernetes sets this from `metadata.name`. |
| `INTERNAL_BIND_ADDR` | `127.0.0.1:3002` | Bind address for the private authenticated listener. Multi-replica deployments must bind it to a peer-reachable interface, such as `0.0.0.0:3002` inside a pod network. |
| `INTERNAL_ADVERTISE_URL` | `http://{POD_IP}:3002`, then `http://{HOSTNAME}:3002` | Peer-reachable base URL stored in ownership records. Set an internal pod IP or private DNS URL. Never set the public ingress URL. |
| `INTERNAL_DISPATCH_HMAC_KEY` | derived from `ENCRYPTION_KEY` | Optional 32-byte HMAC key as 64 hex characters. All replicas must use the same value. Set it explicitly when the local encryption key is unavailable, such as with a KMS key provider. |
| `INTERNAL_AUTH_MAX_SKEW_SECS` | `30` | Maximum clock skew accepted by the internal request signature verifier. |
| `INTERNAL_NONCE_TTL_SECS` | `120` | MongoDB replay-record lifetime for internal request nonces. This must exceed twice `INTERNAL_AUTH_MAX_SKEW_SECS`. |
| `INTERNAL_DUPLEX_HANDSHAKE_TIMEOUT_SECS` | `5` | Maximum time an authenticated internal WebSocket may wait before sending its signed opening envelope. |
| `NODE_OWNER_LEASE_TTL_SECS` | `90` | Lifetime of a node socket owner record without renewal. |
| `NODE_OWNER_LEASE_RENEW_SECS` | `30` | Renewal interval for node socket owner records. Keep this below `NODE_OWNER_LEASE_TTL_SECS`. |
| `CLUSTER_LEASE_TTL_SECS` | `30` | Default lifetime for MongoDB leases, including OAuth refresh claims and Telegram polling leadership. |
| `CLUSTER_LEASE_RENEW_SECS` | `10` | Default renewal interval for renewable cluster leases. Keep this below `CLUSTER_LEASE_TTL_SECS`. |
| `CLUSTER_SLOT_TTL_SECS` | `30` | Lifetime of a global WebSocket or SSH capacity slot without renewal. |
| `CLUSTER_SLOT_RENEW_SECS` | `10` | Renewal interval for occupied capacity slots. Losing renewal cancels the associated session. |
| `MCP_NOTIFICATION_POLL_INTERVAL_MS` | `250` | Poll interval used by an SSE holder to read its durable MongoDB notification outbox. |
| `MCP_NOTIFICATION_TTL_SECS` | `86400` | Retention period for delivered or abandoned MCP outbox notifications. |

Kubernetes also injects `POD_NAME` and `POD_IP` through the downward API. The
manifest copies `POD_NAME` to `INSTANCE_NAME` and builds
`INTERNAL_ADVERTISE_URL` from `POD_IP`. These two downward API variables are
deployment inputs, not standalone NyxID settings.

## Assistant Diagnostics

The Aevatar assistant chat wire-log diagnostic has no environment variable. It
is gated by the `experimental:aevatar-chat-wire-log` runtime feature flag
(default off), toggled platform-wide, per org cohort, or per user through the
platform-admin feature-flag API with no redeploy. See
`docs/assistant-wire-log.md`.

The standalone remote credential accept routes are backend-served. In split
frontend/backend deployments, reverse proxies must route
`/nodes/{node_id}/credentials/pending/{pending_id}/accept`,
`/nodes/credentials/pending/{pending_id}/fan-out/accept`, and
`/credential-accept/assets/*` to the backend. Frontend callers build the
absolute accept URL from `runtime-config.api_base_url`; see
[RELEASE_INTEGRITY.md](RELEASE_INTEGRITY.md#standalone-accept-page).

## Database

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_MAX_CONNECTIONS` | `10` | Connection pool max size |

## Billing Meter and Lago Sink

NyxID writes a durable `usage_meter` ledger, can push finalized rows into Lago, and keeps wallet and entitlement state fresh with signed Lago webhooks plus the periodic reconcile sweep as a backstop.

| Variable | Default | Description |
|----------|---------|-------------|
| `BILLING_ENABLED` | `false` | Enables platform usage capture and the prepaid wallet charging gate. When Lago is configured, owner wallets are provisioned idempotently on startup backfill, wallet access, registration best-effort, and first proxy use. |
| `LAGO_API_URL` | *(empty)* | Lago API URL for the P2 sink. May include or omit `/api/v1`. |
| `LAGO_API_KEY` | *(empty)* | Lago API bearer key for NyxID-to-Lago calls; redacted from config debug output. |
| `LAGO_PLAN_CODE` | `starter` | Lago plan code used by owner wallet provisioning/backfill when creating the owner's subscription. |
| `LAGO_PAYMENT_PROVIDER_CODE` | *(empty)* | Code of the Lago Stripe payment provider connection. When set, newly created Lago customers are linked to it (`sync_with_provider: true`) so `POST /api/v1/billing/topup` can generate checkout URLs. Unset leaves customers unlinked and top-ups fail with `no_linked_payment_provider`; existing customers must be linked manually in the Lago dashboard. |
| `LAGO_WEBHOOK_SECRET` | *(empty)* | Lago webhook verification secret for `POST /api/v1/webhooks/lago`; redacted from config debug output. Required to accept Lago-originated wallet/subscription updates. |
| `BILLING_RECONCILE_INTERVAL_SECS` | `300` | Reconcile sweep interval. Set `0` to disable event push/reconcile sweeps. |
| `BILLING_RATE_CACHE_TTL_SECS` | `900` | Maximum age of a read-only rate used for reservation sizing. A missing or older rate rejects a billable request before forwarding. The reconcile sweep mirrors the Lago plan's standard charges into the cache each cycle (and once at startup), so the default works when the sweep is enabled; raise it only when `BILLING_RECONCILE_INTERVAL_SECS=0`. |
| `BILLING_RESERVATION_ABANDON_SECS` | `600` | Grace before never-forwarded reserved rows are marked `abandoned`. |
| `BILLING_DEFAULT_OVERDRAFT_CAP_CREDITS` | `0` | Hard default overdraft cap copied to newly provisioned wallets. The shared-store reservation gate enforces the wallet's cap atomically. |
| `BILLING_FAIL_CLOSED` | `false` | Incident kill switch. When `BILLING_ENABLED=true`, rejects billable forwarding even when Lago is healthy; when billing is disabled it has no effect. |
| `BILLING_RESALE_ENABLED` | `false` | Explicit opt-in for the dormant catalog resale layer. Resale still also requires `ServiceBilling.resale_billable=true` and final `CredentialClass::NyxidManagedMaster`. |

### Billing flag matrix

`BILLING_ENABLED`, `BILLING_RESALE_ENABLED`, and `BILLING_FAIL_CLOSED` are independent. `BILLING_ENABLED` controls platform metering, wallet provisioning, and the reservation gate; it does not implicitly enable catalog resale. `BILLING_RESALE_ENABLED` controls only the resale ledger layer, and `BILLING_FAIL_CLOSED` is consulted only when `BILLING_ENABLED=true`.

| `BILLING_ENABLED` | `BILLING_RESALE_ENABLED` | `BILLING_FAIL_CLOSED` | Effective behavior |
|---|---|---|---|
| `false` | `false` | `false` | Billing is dark: no platform or resale `usage_meter` rows and no wallet gate. |
| `false` | `false` | `true` | Same as above; the fail-closed switch is inactive while billing is disabled. |
| `false` | `true` | `false` | Resale shadow capture only for an eligible NyxID-managed credential. There is no wallet provisioning or reservation gate. |
| `false` | `true` | `true` | Same resale shadow-capture mode; the fail-closed switch remains inactive. |
| `true` | `false` | `false` | Platform metering and wallet reservation are active; catalog resale is off. |
| `true` | `true` | `false` | Platform metering and eligible catalog resale are both active and cross the same durable settlement boundary. |
| `true` | `false` | `true` | Billable forwarding is stopped with a billing-provider error. Use only as an incident kill switch. |
| `true` | `true` | `true` | Same stop behavior; no platform or resale request is forwarded. |

The Lago client is configured only when both `LAGO_API_URL` and `LAGO_API_KEY` are non-empty. With `BILLING_ENABLED=true`, `BILLING_FAIL_CLOSED=false`, and no Lago client, existing chargeable wallets degrade to unreserved meter-only capture and missing wallets cannot be auto-provisioned. `LAGO_WEBHOOK_SECRET` is independent of outbound client configuration: it authenticates inbound `/api/v1/webhooks/lago` calls and must be set to accept wallet or entitlement updates. `LAGO_PLAN_CODE` selects the subscription created during provisioning; `BILLING_RECONCILE_INTERVAL_SECS=0` disables both usage push and settlement recovery sweeps.

Admins may set an exact `credits_per_unit` price in a catalog service's `billing.platform_pricing` block. NyxID owns those prices and synchronizes a stable `platform_svc_{slug}` sum metric plus its standard charge onto `LAGO_PLAN_CODE`. Lago plan updates always round-trip the complete plan and every existing charge because `PUT /plans/{code}` replaces the charge array; existing charge ids and unrelated pricing must never be omitted. The saved row records `pending`, `synced`, or `failed`; failed and pending updates are retried by the reconcile sweep. Traffic switches to the service-specific metric only after synchronization succeeds, so a partial Lago update cannot silently apply a stale local price. Clearing a price immediately restores the legacy metric and persists a cleanup marker until the NyxID charge and local rate-cache row are removed. Catalog services without `platform_pricing` continue using the legacy plan-authored platform metric and rate.

Credit benefits use five collections. `credit_grants` stores one attributable row per recipient. `credit_schedules` stores recurring credit policy, and `credit_schedule_periods` stores derived walk progress. `usage_allowances` stores recurring free-unit definitions. `usage_allowance_periods` stores each owner's consumption and reservations for a UTC window. Platform admins manage grants, schedules, and allowances under `/api/v1/admin/credits`. Operators may read those admin endpoints but cannot mutate them. Flagged users read active balances from `GET /api/v1/billing/grants` and `GET /api/v1/billing/allowances`. An authorized organization member may pass `owner_id` to read the organization's benefits. Wallet mutations remain restricted to organization admins.

An "all users" one-shot grant snapshots active person and organization owners at issuance. A recurring schedule takes that snapshot when it claims the UTC period. An "all users" allowance applies dynamically as each owner spends. A scheduled grant's UUID v5 `_id` is derived from the schedule ID, the period start, and the recipient ID. That `_id` is the disbursement identity. Period progress and leases do not decide whether NyxID paid a recipient. Retries converge on the same ordinary grant and the existing `grant-issued:{grant_id}` ledger key. Schedule catch-up opens only the current window and never backfills elapsed credits. A paused schedule finishes an open period but opens no later period.

One-shot issuance journals at most 50 recipients inline to bound a platform-wide request. Scheduled walks use the reconcile sweep's recipient budget. Unjournaled grants remain unspendable until recovery confirms their issuance entries. Credit schedules use `BILLING_RECONCILE_INTERVAL_SECS`; they add no environment variable.

Funding order is free allowance units, promotional grant microcredits (soonest expiry first), then wallet credits. The reservation gate holds estimated allowance units and grant value, but settlement applies actual metric quantity and releases any excess hold. Grant reservations admitted before expiry remain valid; otherwise expiry is checked at the instant of reservation. Daily windows start at 00:00 UTC, weekly windows at Monday 00:00 UTC, monthly windows on the first day at 00:00 UTC, and one-time allowances never reset. Only the wallet-funded fraction of a finalized usage row is pushed to Lago. Fully benefit-funded rows are acknowledged locally, and Lago drift comparison sums that same wallet-funded decimal quantity, preventing grants or allowances from becoming a second invoice charge.

Purchased credits expire 365 days after the Lago wallet transaction settles. Lago v1.50 exposes a wallet-level `expiration_at`, but that expires the entire wallet at one instant and cannot represent independently rolling purchases, so NyxID performs FIFO per-purchase expiry from traceable `remaining_credit_amount` values (with a conservative legacy wallet-balance fallback) and debits Lago with `voided_credits`. A durable operation embedded in `billing_wallet` holds expiring credits out of availability, recovers a provider debit by its unique operation name after a crash, reads back the exact Lago balance, updates top-up history, and confirms every `topup_expired` ledger entry before clearing. The existing reconcile interval drives grant expiry and purchased-credit expiry; no new environment variable is required. Keep `BILLING_RECONCILE_INTERVAL_SECS` non-zero in production.

The intended production configuration is `BILLING_ENABLED=true`, `BILLING_FAIL_CLOSED=false`, both Lago client variables set, `LAGO_WEBHOOK_SECRET` set, a non-zero reconcile interval, and fresh rate-cache rows for every enabled metric. Set `BILLING_RESALE_ENABLED=true` only when catalog resale is intentionally offered. A rollout may use the resale-only shadow mode to inspect ledger quantities, but it does not enforce funding and must not be represented as charging.

Billing policy for public and relay paths:

- Public proxy (`/public/s/{slug}`) is block-not-meter. It has no `AuthUser`, API key, or wallet owner, so enabled anonymous endpoints cannot be combined with `ServiceBilling.resale_billable=true`; writes and runtime reads reject that shape with `AnonymousIncompatibleBilling` (`11304`) before forwarding.
- Public MCP (`/public/mcp`) is discovery-only. `tools/list` may describe safe anonymous endpoints, but `tools/call` returns `"Public MCP tool execution is not supported"` and never forwards traffic.
- Oracle relay (`/api/v1/oracle`) is explicitly exempt. Tasks run on user-supplied browser worker capacity; NyxID does not supply downstream model credentials, tokens, or paid compute on that path. If NyxID-hosted Oracle workers or NyxID-paid model capacity are introduced later, they must attach a `BillingRouteContext` before dispatch.

Configure Lago to send webhooks to `<BASE_URL>/api/v1/webhooks/lago` with the same shared secret as `LAGO_WEBHOOK_SECRET`. NyxID verifies `X-Lago-Signature` over the raw request body before processing and uses `X-Lago-Unique-Key` only as metadata. Wallet events refresh the local wallet balance from Lago and clear accounted `pending_lago_debits`; subscription or entitlement events invalidate the local billing decision marker. The reconcile sweep remains enabled for missed or delayed webhooks unless `BILLING_RECONCILE_INTERVAL_SECS=0`.

`POST /api/v1/billing/wallet` provisions the owner in Lago and creates a local `billing_wallet` cache idempotently. `POST /api/v1/billing/topup` creates a Lago wallet transaction, then uses Lago's documented `POST /api/v1/wallet_transactions/{lago_id}/payment_url` endpoint to obtain the provider-hosted Stripe checkout URL. Personal owners may manage their own wallet; only org admins may provision or top up an org wallet. Org members may continue consuming authorized org resources, which charge the org wallet, but cannot mutate it. NyxID never directly increments local credits; local balance changes only after Lago webhook/reconcile confirms the wallet balance.

Before enabling paid top-ups for users, run a deployed sandbox verification:

```bash
nyxid billing verify-topup-flow --amount-credits 1 --open
```

Complete the Stripe sandbox checkout opened by the command. The verifier passes only when the reconciled wallet balance increases by exactly the paid amount and the checkout response is Stripe-backed.

## JWT

| Variable | Default | Description |
|----------|---------|-------------|
| `JWT_PRIVATE_KEY_PATH` | `keys/private.pem` | Path to RSA private key PEM file |
| `JWT_PUBLIC_KEY_PATH` | `keys/public.pem` | Path to RSA public key PEM file |
| `JWT_ISSUER` | `nyxid` | JWT `iss` claim value |
| `JWT_ACCESS_TTL_SECS` | `900` (15 min) | Access token lifetime in seconds |
| `JWT_REFRESH_TTL_SECS` | `604800` (7 days) | Refresh token lifetime in seconds |
| `JWT_RELAY_REPLY_TTL_SECS` | `1800` (30 min) | Lifetime of the per-callback reply token issued with channel-relay inbound callbacks (see [CHANNEL_BOT_RELAY.md](CHANNEL_BOT_RELAY.md#reply-token)). Tokens are single-use, scoped to one inbound message + conversation + agent, and cannot be used against other NyxID endpoints. |
| `JWT_RELAY_CALLBACK_TTL_SECS` | `300` (5 min) | Lifetime of the signed channel-relay callback JWT sent in `X-NyxID-Callback-Token`. |
| `JWT_RELAY_ACCESS_TTL_SECS` | `300` (5 min) | Lifetime of the `X-NyxID-User-Token` relay access token shipped to a bot callback URL. Kept short (vs. the 900s general access token) because it is a first-party bearer credential that leaves NyxID's trust boundary. It is usable only on proxy/LLM surfaces (rejected elsewhere), inherits the originating agent key's service/node allowlist, and is invalidated when that agent key is revoked. |
| `JWT_ASSISTANT_FORWARD_TTL_SECS` | `300` (5 min) | **LEGACY / tombstone.** Was the TTL of the retired `assistant_forward` marker token. Live assistant capability uses a standard delegated access token whose 300-second lifetime is the compile-time constant `crypto::jwt::MCP_DELEGATION_TOKEN_TTL_SECS`; there is no environment variable for that lifetime, so setting this variable changes no live assistant token. See [Assistant Chat Architecture](chat/01-architecture.md#authorization-is-not-caller-passthrough). |
| `SA_TOKEN_TTL_SECS` | `3600` (1 hour) | Service account token lifetime in seconds |

In development mode, RSA keys are auto-generated if the files do not exist. In production, you must provide pre-generated keys:

```bash
openssl genrsa -out keys/private.pem 4096
openssl rsa -in keys/private.pem -pubout -out keys/public.pem
chmod 600 keys/private.pem
```

## OAuth Broker Bindings (Optional, V2 hardening)

Header-forwarded mTLS for certificate-bound broker access tokens (RFC 8705 §3). DPoP support (RFC 9449), AAD-bound encryption, chain-follow retry, RFC 7662 introspection, and revocation webhooks need no environment configuration. Two broker hardening gates are default-off for compatibility while existing broker clients adopt sender constraints and admin provisioning. These env values are startup defaults only: platform admins can override or clear them at runtime through Admin → OAuth Clients → Broker Rollout Policy (`GET/PATCH /api/v1/admin/settings/broker`). A MongoDB override wins over the env default until it is reset to `null`; with no override and no env var, behavior remains default-off. The process that handles an admin update refreshes immediately, and other backend replicas refresh their in-memory snapshot from MongoDB on a short background interval, so enforcement paths do not add per-request database reads.

> **Operational caveat:** If the `platform_settings` document is manually deleted or reset to a revision below a process's cached broker-policy revision, that process retains its last-known-good policy until it is restarted.

| Variable | Default | Description |
|----------|---------|-------------|
| `MTLS_CLIENT_CERT_HEADER` | *(empty)* | HTTP header name carrying the URL-encoded client certificate PEM forwarded by an upstream mTLS-terminating reverse proxy. When set AND a broker token-exchange call (`POST /oauth/token` with `subject_token_type=urn:nyxid:params:oauth:token-type:binding-id`) carries that header, NyxID parses the cert, computes its SHA-256 thumbprint over the DER, and binds the issued access_token to it via the `cnf.x5t#S256` claim. The `mw/auth.rs` middleware then requires the same cert header on every API call using that token and rejects with 401 on mismatch. **OFF BY DEFAULT.** Operators MUST set this AND configure their proxy to strip the header from external requests before forwarding — otherwise an attacker can inject the header and forge a binding. Common values: `X-Client-Cert` (nginx with `proxy_set_header X-Client-Cert $ssl_client_escaped_cert;`), `x-amzn-mtls-clientcert` (AWS ALB), `x-forwarded-client-cert` (Envoy). DPoP (sent by the client itself, no proxy trust required) takes precedence when both headers are present. |
| `BROKER_REQUIRE_SENDER_CONSTRAINT` | `false` | Startup default for sender-constraint enforcement. When the effective value is `false`, legacy unpinned broker bindings continue to exchange as bearer-compatible credentials, while new bindings are pinned if the client presents DPoP or trusted mTLS during authorization-code exchange. When the effective value is `true`, NyxID refuses to mint an unpinned broker binding and refuses to exchange any existing unpinned binding. Flip only after broker clients (for example aevatar) send DPoP or trusted mTLS on both authorization-code and broker token-exchange calls. Runtime DB override wins over this env value. |
| `BROKER_REQUIRE_ADMIN_CAPABILITY` | `false` | Startup default for admin-provisioned broker capability. When the effective value is `false`, the legacy `urn:nyxid:scope:broker_binding` scope trigger and authenticated developer-app self-service `broker_capability_enabled=true` remain accepted for compatibility. When the effective value is `true`, broker capability requires the admin-managed `OAuthClient.broker_capability_enabled` flag: anonymous DCR broker scope requests are rejected, developer-app create/update can set the flag only for platform admins, and broker detection ignores the scope trigger. Ops migration: have a platform admin set `broker_capability_enabled=true` for the broker client through Admin → OAuth Clients (or `PATCH /api/v1/admin/oauth-clients/{client_id}`), verify it no longer depends on the scope trigger, then flip the runtime broker setting. Runtime DB override wins over this env value. |

## Rate Limiting

| Variable | Default | Description |
|----------|---------|-------------|
| `RATE_LIMIT_PER_SECOND` | `10` | Cluster-wide sustained request rate |
| `RATE_LIMIT_BURST` | `30` | Cluster-wide burst capacity and per-IP limit |
| `PLATFORM_SERVICE_RATE_LIMIT_PER_SECOND` | `0` | Sustained requests/second per user for each platform-credentialed service. Defaults to `0` (disabled): enabling a cap on a shared credential is a deliberate operator decision taken after observing real traffic, so it never arrives with a deploy. |
| `PLATFORM_SERVICE_RATE_LIMIT_BURST` | `10` | Burst capacity per user for each platform-credentialed service. |
| `PLATFORM_REQUIRE_OPERATION_POLICY` | `false` | When true, a platform-credentialed catalog row with no `proxy_operation_policy` is refused on actor-addressed paths (`/proxy/s/{slug}`, `/llm/*`). Ships **disabled** so deploying changes no existing behaviour; enable per environment once every such row either carries a policy or is confirmed to receive no actor-addressed traffic. Server-chosen surfaces (the assistant) are unaffected either way — they cannot name an operation, so a policy has no meaning there. |
| `TRUSTED_PROXY_IPS` | *(empty)* | Comma-separated reverse-proxy IPv4/IPv6 addresses or CIDR ranges. Bare addresses mean `/32` (IPv4) or `/128` (IPv6); IPv4-mapped IPv6 addresses are normalized to IPv4. **Only list proxies configured to overwrite client-supplied forwarded headers.** From an allowlisted peer, resolution prefers `CF-Connecting-IP`, then scans `X-Forwarded-For` right-to-left while skipping trusted proxy hops, then uses `X-Real-IP`, then the TCP peer. `CF-Connecting-IP` is the primary Cloudflare path because it does not depend on a complete proxy-hop list. The XFF fallback requires every hop to be listed, including Cloudflare's published IPv4 and IPv6 ranges when Cloudflare is in front; otherwise the rightmost unlisted Cloudflare edge becomes the apparent client and rate-limit key. From an untrusted peer, strict public/device-login paths ignore all forwarded headers. The global limiter and node WebSocket attribution retain their legacy XFF-first behavior only while this setting is empty, then switch to the trusted resolver when configured. Invalid entries are dropped with a warning. Until this is set behind an internal ingress, requester IP and country are unavailable and strict public per-IP buckets can collapse to the ingress peer. |

Deploy the resolver code before changing `TRUSTED_PROXY_IPS`. The code-only deploy is backward compatible for the global limiter and node WebSocket path. Setting the variable is the activation step: Cloudflare client attribution becomes verified, auth-device request/poll/preview limits key by the actual client, and the global/WS paths stop accepting forwarded headers from peers outside the allowlist.

Before enabling trusted proxy attribution in production:

1. Verify the ingress overwrites or strips all client-supplied `CF-Connecting-IP`, `X-Forwarded-For`, `X-Real-IP`, and `CF-IPCountry` headers.
2. Prefer the narrow actual ingress CIDR over broad private ranges. The example `10.0.0.0/8,172.16.0.0/12,192.168.0.0/16` is a starting point only when the ingress network cannot yet be narrowed.
3. Add every proxy hop needed by the XFF fallback. When Cloudflare is in front, include its current published IPv4 and IPv6 ranges as well as the private ingress range. Keep those ranges synchronized with Cloudflare. `CF-Connecting-IP` remains the preferred path because it is independent of this complete-hop requirement.
4. Confirm the origin cannot be reached around Cloudflare. If direct-to-origin traffic is possible, ensure it cannot reach an allowlisted ingress peer.
5. After enabling, initiate a device login while sending a forged `CF-Connecting-IP`, then confirm `/auth/device/preview` does not echo the forged address. Repeat with forged `X-Forwarded-For`, `X-Real-IP`, and `CF-IPCountry` values before treating requester attribution as verified.

For richer requester recognition, enable Cloudflare's **Add visitor location headers**
managed transform in the Cloudflare dashboard. NyxID reads `CF-IPCity`, `CF-Region`,
`CF-IPContinent`, and `CF-Timezone` only when the origin peer matches
`TRUSTED_PROXY_IPS`, under the same trust gate as `CF-IPCountry`. It intentionally
does not retain Cloudflare latitude, longitude, postal-code, or metro-code headers;
city and region are sufficient for a sign-in recognition check without storing more
precise location data. If the transform is disabled or a particular header is absent,
the approval screen degrades cleanly to country-only attribution. The ingress header
overwrite/strip checklist above still applies to every enabled location header.

## CLI Remote Pairing (Optional)

The `nyxid` CLI's wizard-style commands (e.g. `nyxid service add`, `nyxid api-key create`, `nyxid node register-token`) can hand off to a browser on another device via a short pairing code. Codes are 8 Crockford characters (~2^40 space) and live for 15 minutes; the backend keys the stored hash with an HMAC so a MongoDB snapshot alone cannot brute-force them offline.

| Variable | Default | Description |
|----------|---------|-------------|
| `CLI_PAIRING_HMAC_KEY` | *(derived from `ENCRYPTION_KEY`)* | Explicit 32-byte HMAC key (64 hex chars) for `CliPairing.code_hash`. Generate with `openssl rand -hex 32`. Set this in multi-instance deployments where `ENCRYPTION_KEY` is not configured (for example `KEY_PROVIDER=aws-kms` or `gcp-kms`) so every backend worker produces the same HMAC output for a given code. |

### Key selection rules

The backend picks the key at startup using the first match:

1. `CLI_PAIRING_HMAC_KEY` if set (must be 64 hex chars).
2. Derived from `ENCRYPTION_KEY` via HMAC-SHA256 with domain-separated label `nyxid:cli-pairing-code-hmac-v1`. Stable across restarts and workers that share `ENCRYPTION_KEY`. This is the expected path for the local AES provider.
3. Derived from the **JWT private key** file contents via HMAC-SHA256 with a distinct domain-separated label (`...-hmac-v1:jwt`). Lets `KEY_PROVIDER=aws-kms` or `gcp-kms` deployments boot without requiring operators to configure `CLI_PAIRING_HMAC_KEY` up front — and because the same JWT PEM is deployed to every worker, this derivation is stable across a cluster (no sticky-session footgun).

If all three sources are missing, the backend refuses to start with a clear error message pointing at this section. In practice that branch is unreachable because the JWT private key is already required at startup.

### When you must set it explicitly

- You want to rotate the pairing-code HMAC independently of both `ENCRYPTION_KEY` and the JWT signing key (e.g. to rotate it without rotating JWTs).
- Any deployment policy that forbids reusing key material across purposes.

Otherwise the automatic derivation chain is safe: local deployments derive from `ENCRYPTION_KEY`, KMS deployments derive from the JWT signing key.

## Audit Log Hash Chain (Optional)

NyxID audit-log rows created after this feature is enabled are tamper-evident. Each chained row carries `seq`, `prev_hash`, and `entry_hash`; `entry_hash` is an HMAC-SHA256 over a canonical encoding of the row content and the previous row's hash. The HMAC key stays in process memory and is never written to MongoDB, so a database-only attacker cannot recompute a valid chain after editing, deleting, inserting, or reordering chained rows.

| Variable | Default | Description |
|----------|---------|-------------|
| `AUDIT_CHAIN_HMAC_KEY` | *(derived from `ENCRYPTION_KEY`, then JWT private key)* | Explicit 32-byte HMAC key (64 hex chars) for audit-log hash chaining. Generate with `openssl rand -hex 32`. Set this when you need to rotate the audit-chain key independently of both `ENCRYPTION_KEY` and the JWT signing key, or when deployment policy forbids deriving secondary HMAC keys from those sources. |

### Key selection rules

The backend picks the audit-chain key at startup using the first match:

1. `AUDIT_CHAIN_HMAC_KEY` if set (must be 64 hex chars).
2. Derived from `ENCRYPTION_KEY` via HMAC-SHA256 with the domain-separated `audit-chain` label.
3. Derived from the JWT private key file contents with the same domain-separated label.

Legacy rows without `seq` are not backfilled. The verifier reports them as `pre_chain_count` rather than claiming historical integrity. This v1 chain does not detect tail truncation: deleting the newest N rows leaves a valid shorter chain until `(head_seq, head_hash)` is anchored outside MongoDB.

## Billing Ledger Hash Chain (Optional)

Money-moving billing events (charged usage settlements, provider-confirmed top-up checkouts, paid credits landing on a wallet, grant issuance/consumption/expiry/revocation, and purchased-credit expiry) are journaled to the append-only `billing_ledger` collection with the same hash-chain construction as the audit log. The operational billing rows stay mutable by design; the ledger is what makes their money-moving history tamper-evident. Grant issuance is not spendable until its deduplicated ledger row is durable; consumption keeps its resource settlement lock until journal confirmation; issuance and terminal-event markers are retried by reconciliation. Purchased-credit expiry similarly retains its wallet operation until all per-purchase ledger rows exist. Verify via `GET /api/v1/admin/billing-ledger/verify`.

| Variable | Default | Description |
|----------|---------|-------------|
| `BILLING_LEDGER_HMAC_KEY` | *(derived from `ENCRYPTION_KEY`, then JWT private key)* | Explicit 32-byte HMAC key (64 hex chars) for billing-ledger hash chaining. Same key-selection rules as `AUDIT_CHAIN_HMAC_KEY`, with the domain-separated `billing-ledger` label, so the two chains never share a key even when both derive from `ENCRYPTION_KEY`. |

Ledger appends are best-effort relative to billing itself: a ledger write failure is logged and never fails or rolls back a settlement or top-up.

### Automatic verification

Both chains are re-verified automatically by a background sweep that walks the chain in rolling 10,000-entry chunks from a persisted cursor, wrapping back to seq 1 after passing the head so old regions are continuously re-covered. Any break is written to the per-chain `chain_verify_status` document (shown on the admin Integrity page), logged at error level every run until it clears, and the billing sweep additionally cross-checks the head anchor. `GET /api/v1/admin/chain-verification` returns the latest state; `POST /api/v1/admin/chain-verification/run` runs one chunk immediately.

| Variable | Default | Description |
|----------|---------|-------------|
| `CHAIN_VERIFY_INTERVAL_SECS` | `3600` | Interval between automatic verification chunks for both chains. `0` disables the sweep (manual verify endpoints still work). |


Unlike the audit chain, the billing ledger detects tail truncation: the reconcile sweep anchors the ledger head `(seq, head_hash)` into the audit chain (event `billing_ledger_head_anchored`) whenever it advances, and the verify endpoint cross-checks the newest anchor against the surviving head. Deleting ledger tail entries past an anchor reports `tail_truncated`; hiding it would additionally require truncating the audit chain back past the anchor, destroying unrelated audit history. Each anchor is also written to the server log (`billing ledger head anchored`), so shipped logs form an external anchor outside MongoDB. Entries newer than the latest anchor (up to one reconcile interval, `BILLING_RECONCILE_INTERVAL_SECS`) remain inside the undetectable window.

## Social Login (Optional)

| Variable | Description |
|----------|-------------|
| `GOOGLE_CLIENT_ID` | Google OAuth client ID |
| `GOOGLE_CLIENT_SECRET` | Google OAuth secret |
| `GITHUB_CLIENT_ID` | GitHub OAuth client ID |
| `GITHUB_CLIENT_SECRET` | GitHub OAuth secret |

### Apple Sign In

Requires all four values. Create a Services ID and key at the [Apple Developer portal](https://developer.apple.com/account/resources/identifiers/list/serviceId).

| Variable | Description |
|----------|-------------|
| `APPLE_CLIENT_ID` | Apple Services ID (e.g. `com.example.nyxid`) |
| `APPLE_TEAM_ID` | Apple Developer Team ID |
| `APPLE_KEY_ID` | Apple Sign In key ID |
| `APPLE_PRIVATE_KEY_PATH` | Path to Apple `.p8` private key file |

## Telegram / Approval System (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `TELEGRAM_BOT_TOKEN` | | Telegram Bot API token (from @BotFather) |
| `TELEGRAM_WEBHOOK_SECRET` | | Secret for verifying Telegram webhook callbacks |
| `TELEGRAM_WEBHOOK_URL` | | Public URL for Telegram webhooks (e.g. `https://auth.nyxid.dev/api/v1/webhooks/telegram`). Omit to use long polling mode. |
| `TELEGRAM_BOT_USERNAME` | | Bot username without @ (for link instructions) |
| `APPROVAL_EXPIRY_INTERVAL_SECS` | `5` | Interval between approval expiry sweeps (seconds) |

The approval system works without Telegram -- users can always approve/reject via the web UI. Telegram delivery requires `TELEGRAM_BOT_TOKEN`.

**Telegram delivery modes:** When `TELEGRAM_WEBHOOK_URL` (and `TELEGRAM_WEBHOOK_SECRET`) are set, the backend registers a webhook with Telegram at startup. When only `TELEGRAM_BOT_TOKEN` is set (no webhook URL), the backend automatically falls back to `getUpdates` long polling -- ideal for local development without ngrok or tunnels.

## Hosted Connect Links

| Variable | Default | Description |
|----------|---------|-------------|
| `CONNECT_LINK_EXPIRY_SWEEP_INTERVAL_SECS` | `60` | Interval between sweeps that claim overdue app-bound connect links and dispatch `connect_link.expired`. Effective deadlines include the pinned OAuth/device finalization grace. `0` disables the sweep; query-time expiry remains active. |

## OAuth Token Refresh (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `OAUTH_REFRESH_SWEEP_INTERVAL_SECS` | `600` (10 min) | Interval between proactive OAuth refresh sweeps. `0` disables the sweep (lazy proxy-time refresh still applies). |
| `CONNECTION_EXPIRY_NOTIFICATIONS` | `true` | Sends a one-time notification when an OAuth connection changes from healthy to unusable. Audit events are always recorded. |
| `OAUTH_REFRESH_SWEEP_WINDOW_SECS` | `900` (15 min) | How far ahead the sweep looks for expiring access tokens. Keep larger than the proxy-time 5-minute refresh buffer so the sweep wins for idle services. |

The backend refreshes OAuth access tokens two ways: **lazily** at proxy time (whenever a request arrives within 5 minutes of expiry) and **proactively** via this background sweep. The sweep keeps multi-connection OAuth access tokens (Google / Lark / GitHub BYO etc.) warm even for services that aren't proxied often, and surfaces a dead refresh token as `status: "failed"` promptly instead of on the user's next proxy attempt.

The sweep only refreshes the short-lived **access** token. It does **not** extend **refresh**-token lifetime, so it cannot prevent these re-auth causes:

- A Google OAuth app left in **"Testing"** publishing status expires its refresh tokens after 7 days regardless. Publish the app (Google Cloud Console → OAuth consent screen → Publish) to fix.
- A connection authorized before refresh tokens were issued (no `access_type=offline` consent) has no refresh token to use. Re-add the connection once to obtain one.

## Trigger Ingress

| Variable | Default | Description |
|----------|---------|-------------|
| `TRIGGER_RATE_LIMIT_PER_SECOND` | `10` | Sustained public ingress rate allowed per trigger. |
| `TRIGGER_RATE_LIMIT_BURST` | `20` | Per-trigger token bucket capacity. |
| `TRIGGER_PAYLOAD_MAX_BYTES` | `262144` | Maximum raw trigger request body size. |
| `TRIGGER_DELIVERY_RETENTION_HOURS` | `72` | Hours to retain encrypted webhook-target envelopes for durable dedup and replay. `0` disables payload storage/replay while metadata remains bounded to 72 hours. |

Webhook-target dedup uses durable delivery records. Agent and notification targets use the shared `CHANNEL_EVENT_DEDUP_CAPACITY` and `CHANNEL_EVENT_DEDUP_TTL_SECS` bounds in a separate per-process cache.

## Mobile Push Notifications (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `FCM_SERVICE_ACCOUNT_PATH` | | Path to Firebase service account JSON file |
| `APNS_KEY_PATH` | | Path to APNs `.p8` private key file |
| `APNS_KEY_ID` | | APNs Key ID (from Apple Developer portal) |
| `APNS_TEAM_ID` | | APNs Team ID (from Apple Developer portal) |
| `APNS_TOPIC` | | APNs topic / iOS app bundle ID (e.g. `dev.nyxid.app`) |
| `APNS_SANDBOX` | `true` in dev, `false` in prod | Use APNs sandbox environment |

FCM and APNs are independent -- configure either or both. Push notifications are sent in parallel alongside Telegram. Invalid device tokens are automatically cleaned up when the push service reports them as unregistered.

## SMTP (Optional)

| Variable | Description |
|----------|-------------|
| `SMTP_HOST` | SMTP server hostname |
| `SMTP_PORT` | SMTP server port |
| `SMTP_USERNAME` | SMTP authentication username |
| `SMTP_PASSWORD` | SMTP authentication password |
| `SMTP_FROM_ADDRESS` | Sender address for outbound email |

For development, Mailpit is provided via Docker Compose (SMTP on `localhost:1025`, web UI at `http://localhost:8025`).

## Credential Nodes (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `NODE_HEARTBEAT_INTERVAL_SECS` | `30` | Heartbeat ping interval to connected nodes |
| `NODE_HEARTBEAT_TIMEOUT_SECS` | `90` | Mark node offline after N seconds without heartbeat |
| `NODE_PROXY_TIMEOUT_SECS` | `30` | Timeout for proxy requests routed through nodes |
| `NODE_REGISTRATION_TOKEN_TTL_SECS` | `3600` | Registration token validity (1 hour) |
| `NODE_MAX_PER_USER` | `10` | Maximum nodes per user |
| `NODE_MAX_WS_CONNECTIONS` | `100` | Maximum concurrent node WebSocket connections per replica. This protects each process's file descriptors and memory. |
| `NODE_MAX_STREAM_DURATION_SECS` | `300` | Maximum duration for streaming proxy responses |
| `NODE_HMAC_SIGNING_ENABLED` | `true` | Enable HMAC request signing for node proxy requests |

## Proxy (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `PROXY_MAX_BODY_SIZE` | `104857600` | Maximum request body size for authenticated proxy and MCP routes in bytes (100 MiB). Direct and node-routed proxy requests share this raw-body cap; upgraded node agents advertise their capacity, while legacy agents are limited to 11 MiB because their WebSocket frame cap predates capacity negotiation. |
| `LLM_MAX_BODY_SIZE` | `10485760` | Maximum request body size for `/api/v1/llm/*` provider and gateway routes in bytes (10 MiB). |
| `PUBLIC_PROXY_MAX_BODY_SIZE` | `1048576` | Maximum request body size for anonymous public proxy and public MCP routes in bytes (1 MiB). |
| `PUBLIC_PROXY_RATE_LIMIT_PER_MINUTE` | `60` | Dedicated per-IP rate limit for `/public/s/{slug}/{path}` anonymous proxy requests. Honors `TRUSTED_PROXY_IPS` before trusting forwarded client IP headers. |
| `PUBLIC_MCP_RATE_LIMIT_PER_MINUTE` | `30` | Dedicated per-IP rate limit for `POST /public/mcp` anonymous MCP discovery requests. Honors `TRUSTED_PROXY_IPS` before trusting forwarded client IP headers. |
| `PROXY_STREAM_IDLE_TIMEOUT_SECS` | `60` | Terminate a streamed proxy response if no chunk arrives within N seconds |
| `WS_PASSTHROUGH_MAX_CONNECTIONS` | `200` | Maximum concurrent WebSocket passthrough connections across the cluster. MongoDB capacity slots enforce the global cap. |

Other forwarding surfaces remain intentionally fixed and bounded: assistant direct/chat
requests are capped at 256 KiB, SSH exec JSON requests at 64 KiB, and Oracle consumer
and worker payloads at 16 MiB. Ordinary API JSON extractors retain the app-wide 1 MiB cap.
All manual forwarding limits return the structured `request_body_too_large` error (HTTP
413, code 11700) and state the effective byte limit.

## SSH Tunneling (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `SSH_MAX_SESSIONS_PER_USER` | `4` | Maximum concurrent SSH tunnels, browser terminals, and exec operations per authenticated user across the cluster. MongoDB capacity slots enforce the cap. |
| `SSH_CONNECT_TIMEOUT_SECS` | `10` | Timeout for connecting to the downstream SSH target |
| `SSH_MAX_TUNNEL_DURATION_SECS` | `3600` | Maximum duration for a single SSH tunnel before forced close |

## Registration Gate

| Variable | Default | Description |
|----------|---------|-------------|
| `INVITE_CODE_REQUIRED` | `true` | Gate new-user registration behind invite codes. Set to `false` for public registration. Accepts: `true`/`false`, `1`/`0`, `yes`/`no`, `on`/`off`. |
| `EMAIL_AUTH_ENABLED` | `false` | Show the email/password auth UI on `/login` and `/register` and accept `POST /api/v1/auth/register`. Defaults to **false** (SSO-only). The self-host quickstart in `README.md` writes this to `true` automatically. The login API is never gated — existing users can always authenticate via direct API call even when the UI is hidden. Accepts: `true`/`1`/`yes`/`on` → enabled; anything else → disabled. |

## Channel Bot Relay (Deprecated)

> **Deprecated:** These vars apply to the legacy channel bot relay flow (see [#191](https://github.com/ChronoAIProject/NyxID/issues/191)). New deployments should use bot-as-service connections instead (`api-telegram-bot`, `api-lark-bot`, `api-feishu-bot`, `api-discord-bot`).

| Variable | Default | Description |
|----------|---------|-------------|
| `CHANNEL_RELAY_CALLBACK_TIMEOUT_SECS` | `30` | HTTP timeout for agent callback requests |
| `CHANNEL_RELAY_MAX_BOTS_PER_USER` | `5` | Maximum bots per user across all platforms |
| `CHANNEL_RELAY_MESSAGE_TTL_DAYS` | `30` | TTL for `channel_messages` auto-cleanup |

## Oracle Relay

See [ORACLE_RELAY.md](ORACLE_RELAY.md) for the full design.

| Variable | Default | Description |
|----------|---------|-------------|
| `ORACLE_TASK_RETENTION_DAYS` | `30` | Days to retain terminal oracle tasks (prompt + response bodies) before MongoDB TTL expiry. Queued/dispatched tasks are never auto-expired. |

## Logging

| Variable | Default | Description |
|----------|---------|-------------|
| `RUST_LOG` | `nyxid=info,tower_http=info` | Tracing filter string |
