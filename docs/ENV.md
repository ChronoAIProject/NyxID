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
| `SA_TOKEN_TTL_SECS` | `3600` (1 hour) | Service account token lifetime in seconds |

In development mode, RSA keys are auto-generated if the files do not exist. In production, you must provide pre-generated keys:

```bash
openssl genrsa -out keys/private.pem 4096
openssl rsa -in keys/private.pem -pubout -out keys/public.pem
chmod 600 keys/private.pem
```

## OAuth Broker Bindings (Optional, V2 hardening)

Header-forwarded mTLS for certificate-bound broker access tokens (RFC 8705 §3). All other V2 broker hardening (DPoP RFC 9449, PAR RFC 9126, AAD-bound encryption, chain-follow retry, RFC 7662 introspection, revocation webhooks) uses compile-time constants and needs no environment configuration — see `skills/nyxid/references/oauth-broker.md` for the integration-side surface.

| Variable | Default | Description |
|----------|---------|-------------|
| `MTLS_CLIENT_CERT_HEADER` | *(empty)* | HTTP header name carrying the URL-encoded client certificate PEM forwarded by an upstream mTLS-terminating reverse proxy. When set AND a broker token-exchange call (`POST /oauth/token` with `subject_token_type=urn:nyxid:params:oauth:token-type:binding-id`) carries that header, NyxID parses the cert, computes its SHA-256 thumbprint over the DER, and binds the issued access_token to it via the `cnf.x5t#S256` claim. The `mw/auth.rs` middleware then requires the same cert header on every API call using that token and rejects with 401 on mismatch. **OFF BY DEFAULT.** Operators MUST set this AND configure their proxy to strip the header from external requests before forwarding — otherwise an attacker can inject the header and forge a binding. Common values: `X-Client-Cert` (nginx with `proxy_set_header X-Client-Cert $ssl_client_escaped_cert;`), `x-amzn-mtls-clientcert` (AWS ALB), `x-forwarded-client-cert` (Envoy). DPoP (sent by the client itself, no proxy trust required) takes precedence when both headers are present. |

## Rate Limiting

| Variable | Default | Description |
|----------|---------|-------------|
| `RATE_LIMIT_PER_SECOND` | `10` | Global rate limit (requests/second) |
| `RATE_LIMIT_BURST` | `30` | Burst capacity and per-IP limit |
| `TRUSTED_PROXY_IPS` | *(empty)* | Comma-separated list of reverse-proxy IP addresses (IPv4 or IPv6) whose `X-Forwarded-For` / `X-Real-IP` headers may be trusted when keying per-IP rate limits (currently: the CLI-pairing claim limiter, 5/60s per client). Leave empty for direct-exposure deployments — the TCP peer is then used directly, and forwarded headers are ignored so they cannot be spoofed. Set to the IPs of your ingress/load balancer when deployed behind nginx, an ALB, Fly.io's proxy, etc., so each end-user gets their own bucket instead of sharing one with every other user that hits the proxy. **Only list proxies you have configured to overwrite client-supplied `X-Forwarded-For` / `X-Real-IP` headers** — otherwise the allowlist extends trust to the original client. Invalid entries are dropped with a warning. |

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

## OAuth Token Refresh (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `OAUTH_REFRESH_SWEEP_INTERVAL_SECS` | `600` (10 min) | Interval between proactive OAuth refresh sweeps. `0` disables the sweep (lazy proxy-time refresh still applies). |
| `OAUTH_REFRESH_SWEEP_WINDOW_SECS` | `900` (15 min) | How far ahead the sweep looks for expiring access tokens. Keep larger than the proxy-time 5-minute refresh buffer so the sweep wins for idle services. |

The backend refreshes OAuth access tokens two ways: **lazily** at proxy time (whenever a request arrives within 5 minutes of expiry) and **proactively** via this background sweep. The sweep keeps multi-connection OAuth access tokens (Google / Lark / GitHub BYO etc.) warm even for services that aren't proxied often, and surfaces a dead refresh token as `status: "failed"` promptly instead of on the user's next proxy attempt.

The sweep only refreshes the short-lived **access** token. It does **not** extend **refresh**-token lifetime, so it cannot prevent these re-auth causes:

- A Google OAuth app left in **"Testing"** publishing status expires its refresh tokens after 7 days regardless. Publish the app (Google Cloud Console → OAuth consent screen → Publish) to fix.
- A connection authorized before refresh tokens were issued (no `access_type=offline` consent) has no refresh token to use. Re-add the connection once to obtain one.

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
| `NODE_MAX_WS_CONNECTIONS` | `100` | Maximum concurrent node WebSocket connections |
| `NODE_MAX_STREAM_DURATION_SECS` | `300` | Maximum duration for streaming proxy responses |
| `NODE_HMAC_SIGNING_ENABLED` | `true` | Enable HMAC request signing for node proxy requests |

## Proxy (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `PROXY_MAX_BODY_SIZE` | `104857600` | Maximum request body size for proxy routes in bytes (100 MB) |
| `PROXY_STREAM_IDLE_TIMEOUT_SECS` | `60` | Terminate a streamed proxy response if no chunk arrives within N seconds |
| `WS_PASSTHROUGH_MAX_CONNECTIONS` | `200` | Maximum concurrent WebSocket passthrough connections |

## SSH Tunneling (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `SSH_MAX_SESSIONS_PER_USER` | `4` | Maximum concurrent SSH tunnel sessions per authenticated user |
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

## Logging

| Variable | Default | Description |
|----------|---------|-------------|
| `RUST_LOG` | `nyxid=info,tower_http=info` | Tracing filter string |
