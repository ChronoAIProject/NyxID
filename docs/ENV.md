# Environment Variables

All configuration is loaded from environment variables. A `.env` file is supported via `dotenvy`.

## Required

| Variable         | Description                                        | Example                                        |
|------------------|----------------------------------------------------|------------------------------------------------|
| `DATABASE_URL`   | MongoDB connection string                          | `mongodb://localhost:27017/nyxid`              |
| `ENCRYPTION_KEY` | 32-byte hex-encoded AES-256 key (64 hex chars)     | Output of `openssl rand -hex 32`               |

## Encryption

| Variable                   | Default | Description                                                          |
|----------------------------|---------|----------------------------------------------------------------------|
| `ENCRYPTION_KEY_PREVIOUS`  | *(none)* | Previous encryption key for zero-downtime key rotation (64 hex chars). Set this to the old `ENCRYPTION_KEY` value when rotating keys. With Phase 2 envelope encryption, KEK rotation only re-wraps per-record DEK blobs (via `rewrap()`) without re-encrypting data. One previous key supported at a time; finish re-wrapping before rotating again. See [SECURITY.md](SECURITY.md#key-rotation) for the full procedure and `/health` decrypt counters. |

## Server

| Variable       | Default                  | Description                          |
|----------------|--------------------------|--------------------------------------|
| `PORT`         | `3001`                   | HTTP listen port                     |
| `BASE_URL`     | `http://localhost:3001`  | Backend base URL (used in JWT `aud`) |
| `FRONTEND_URL` | `http://localhost:3000`  | Frontend origin for CORS             |
| `ENVIRONMENT`  | `development`            | `development`, `staging`, `production` |

## Database

| Variable                   | Default | Description                     |
|----------------------------|---------|---------------------------------|
| `DATABASE_MAX_CONNECTIONS` | `10`    | Connection pool max size        |

## JWT

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

## Rate Limiting

| Variable               | Default | Description                            |
|------------------------|---------|----------------------------------------|
| `RATE_LIMIT_PER_SECOND`| `10`    | Global rate limit (requests/second)    |
| `RATE_LIMIT_BURST`     | `30`    | Burst capacity and per-IP limit        |

## Social Login (Optional)

| Variable               | Description             |
|------------------------|-------------------------|
| `GOOGLE_CLIENT_ID`     | Google OAuth client ID  |
| `GOOGLE_CLIENT_SECRET` | Google OAuth secret     |
| `GITHUB_CLIENT_ID`     | GitHub OAuth client ID  |
| `GITHUB_CLIENT_SECRET` | GitHub OAuth secret     |

## Telegram / Approval System (Optional)

| Variable                         | Default | Description                                      |
|----------------------------------|---------|--------------------------------------------------|
| `TELEGRAM_BOT_TOKEN`             |         | Telegram Bot API token (from @BotFather)         |
| `TELEGRAM_WEBHOOK_SECRET`        |         | Secret for verifying Telegram webhook callbacks  |
| `TELEGRAM_WEBHOOK_URL`           |         | Public URL for Telegram webhooks (e.g. `https://auth.nyxid.dev/api/v1/webhooks/telegram`). Omit to use long polling mode. |
| `TELEGRAM_BOT_USERNAME`          |         | Bot username without @ (for link instructions)   |
| `APPROVAL_EXPIRY_INTERVAL_SECS`  | `5`     | Interval between approval expiry sweeps (seconds)|

The approval system works without Telegram. Users can always approve/reject via the web UI. Telegram delivery requires `TELEGRAM_BOT_TOKEN`.

## Mobile Push Notifications (Optional)

| Variable                         | Default | Description                                          |
|----------------------------------|---------|------------------------------------------------------|
| `FCM_SERVICE_ACCOUNT_PATH`       |         | Path to Firebase service account JSON file           |
| `APNS_KEY_PATH`                  |         | Path to APNs .p8 private key file                    |
| `APNS_KEY_ID`                    |         | APNs Key ID (from Apple Developer portal)            |
| `APNS_TEAM_ID`                   |         | APNs Team ID (from Apple Developer portal)           |
| `APNS_TOPIC`                     |         | APNs topic / iOS app bundle ID (e.g. `dev.nyxid.app`)|
| `APNS_SANDBOX`                   | `true` in dev, `false` in prod | Use APNs sandbox environment |

FCM and APNs are independent. Configure either or both. Push notifications are sent in parallel alongside Telegram. Invalid device tokens are automatically cleaned up when the push service reports them as unregistered.

## SMTP (Optional)

| Variable            | Description                       |
|---------------------|-----------------------------------|
| `SMTP_HOST`         | SMTP server hostname              |
| `SMTP_PORT`         | SMTP server port                  |
| `SMTP_USERNAME`     | SMTP authentication username      |
| `SMTP_PASSWORD`     | SMTP authentication password      |
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

## Proxy Streaming (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `PROXY_MAX_BODY_SIZE` | `104857600` | Maximum request body size for proxy routes in bytes (100 MB) |
| `PROXY_STREAM_IDLE_TIMEOUT_SECS` | `60` | Terminate a streamed proxy response if no chunk arrives within N seconds |

## SSH Tunneling (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `SSH_MAX_SESSIONS_PER_USER` | `4` | Maximum concurrent SSH tunnel sessions per authenticated user |
| `SSH_CONNECT_TIMEOUT_SECS` | `10` | Timeout for connecting NyxID or a node agent to the downstream SSH target |
| `SSH_MAX_TUNNEL_DURATION_SECS` | `3600` | Maximum duration for a single SSH tunnel before NyxID forces it closed |

## Logging

| Variable   | Default                                | Description              |
|------------|----------------------------------------|--------------------------|
| `RUST_LOG` | `nyxid=info,tower_http=info` | Tracing filter string |
