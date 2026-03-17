# NyxID Deployment Guide

This guide covers deploying NyxID in development, staging, and production environments.

---

## Table of Contents

- [Prerequisites](#prerequisites)
- [Local Development](#local-development)
- [Building for Production](#building-for-production)
- [Docker Deployment](#docker-deployment)
- [Environment Configuration](#environment-configuration)
- [Database Setup](#database-setup)
- [RSA Key Management](#rsa-key-management)
- [Reverse Proxy Configuration](#reverse-proxy-configuration)
- [TLS and Certificates](#tls-and-certificates)
- [Frontend Deployment](#frontend-deployment)
- [Health Checks and Monitoring](#health-checks-and-monitoring)
- [Backup and Recovery](#backup-and-recovery)
- [Scaling](#scaling)
- [OIDC Provider Configuration](#oidc-provider-configuration)
- [Initial Admin Setup](#initial-admin-setup)
- [CLI Reference](#cli-reference)
- [MCP Proxy Deployment](#mcp-proxy-deployment)
- [Telegram Bot Setup](#telegram-bot-setup)
- [Security Hardening Checklist](#security-hardening-checklist)
- [Troubleshooting](#troubleshooting)

---

## Prerequisites

| Tool       | Version   | Purpose                              |
|------------|-----------|--------------------------------------|
| Rust       | 1.85+     | Backend compiler (edition 2024)      |
| Node.js    | 20+       | Frontend build tooling               |
| MongoDB    | 7.0+      | Primary database                     |
| Docker     | 24+       | Container runtime (optional)         |
| openssl    | 3.x       | Key generation                       |

---

## Local Development

### 1. Start infrastructure

```bash
docker compose up -d
```

This starts:
- **MongoDB 8.0** on `127.0.0.1:27017` (credentials: `nyxid` / `nyxid_dev_password`)
- **Mailpit** SMTP on port `1025`, web UI on port `8025`

### 2. Configure environment

```bash
cp .env.example .env
```

The default `.env.example` is pre-configured for local development. Replace the placeholder `ENCRYPTION_KEY`:

```bash
# Generate a real encryption key
openssl rand -hex 32
```

Paste the output as `ENCRYPTION_KEY` in `.env`.

### 3. Start the backend

```bash
cargo run --manifest-path backend/Cargo.toml
```

The backend starts on `http://localhost:3001`. In development mode:
- RSA signing keys are auto-generated in `keys/` if missing
- MongoDB collections and indexes are created automatically
- Email verification tokens are logged to the console

### 4. Start the frontend

```bash
cd frontend
npm install
npm run dev
```

The frontend starts on `http://localhost:3000`.

### 5. Verify

```bash
curl http://localhost:3001/health
# {"status":"ok","version":"0.1.0"}
```

---

## Building for Production

### Backend

```bash
# Release build (optimized, no debug symbols)
cargo build --release --manifest-path backend/Cargo.toml

# Binary output
ls -la backend/target/release/nyxid
```

The release binary is a single statically-linked executable (with dynamically linked system libraries). No runtime dependencies beyond the OS.

### Frontend

```bash
cd frontend
npm ci
npm run build

# Output in frontend/dist/
ls -la frontend/dist/
```

The build produces static files (HTML, JS, CSS) ready to serve from any CDN or static file server.

---

## Docker Deployment

Production Dockerfiles are provided for both backend and frontend with multi-stage builds, dependency caching, and non-root users.

### Building Images

```bash
# Build backend (context is the repo root for workspace Cargo.toml)
docker build -f backend/Dockerfile -t nyxid-backend .

# Build frontend
docker build -f frontend/Dockerfile -t nyxid-frontend frontend/
```

The **backend Dockerfile** (`backend/Dockerfile`) uses a two-stage build:
1. `rust:1.85-bookworm` builder with dependency caching (copies manifests first, builds deps, then copies source)
2. `debian:bookworm-slim` runtime with only `ca-certificates` and `curl`, running as non-root user `nyxid`

The **frontend Dockerfile** (`frontend/Dockerfile`) uses a two-stage build:
1. `node:20-alpine` builder with `npm ci` and `npm run build`
2. `nginx:1.27-alpine` runtime serving the SPA with automatic `envsubst` for `BACKEND_URL`

The frontend nginx config (`frontend/nginx.conf.template`) handles:
- SPA fallback (`try_files $uri $uri/ /index.html`)
- Reverse proxy for `/api`, `/oauth`, `/.well-known`, `/health` to the backend
- Gzip compression and cache headers for hashed assets
- Security headers

### Production Docker Compose

A complete production stack is defined in `docker-compose.prod.yml`:

```bash
# Create a production env file
cp .env.example .env.production
# Edit .env.production with real values (ENCRYPTION_KEY, DATABASE_URL, etc.)

# Generate RSA keys if not already present
mkdir -p keys
openssl genrsa -out keys/private.pem 4096
openssl rsa -in keys/private.pem -pubout -out keys/public.pem

# Set MongoDB root password
export MONGO_ROOT_PASSWORD=$(openssl rand -base64 24)

# Start all services
docker compose -f docker-compose.prod.yml up -d
```

The compose file starts three services:
- **backend**: builds from `backend/Dockerfile`, mounts `keys/` read-only, waits for MongoDB health (MCP transport is built into the backend at `/mcp`)
- **frontend**: builds from `frontend/Dockerfile`, proxies API requests to backend via `BACKEND_URL` env var
- **mongodb**: persistent volume, health check via `mongosh`

### Pushing to a Registry

```bash
# Tag and push to your container registry
docker tag nyxid-backend registry.example.com/nyxid/backend:v0.1.0
docker tag nyxid-frontend registry.example.com/nyxid/frontend:v0.1.0

docker push registry.example.com/nyxid/backend:v0.1.0
docker push registry.example.com/nyxid/frontend:v0.1.0
```

---

## Kubernetes Deployment

Kubernetes manifests are provided in the `k8s/` directory for deploying NyxID to a Kubernetes cluster.

### Prerequisites

- Kubernetes cluster (1.27+)
- `kubectl` configured with cluster access
- nginx ingress controller installed
- Container images pushed to an accessible registry
- RSA key pair generated (see [RSA Key Management](#rsa-key-management))

### Manifests Overview

| File | Resource |
|------|----------|
| `k8s/namespace.yaml` | `nyxid` namespace |
| `k8s/configmap.yaml` | Non-secret configuration (URLs, JWT settings, rate limits) |
| `k8s/secrets.yaml` | Secret templates (DB credentials, encryption key, JWT keys) |
| `k8s/backend-deployment.yaml` | Backend Deployment (2 replicas) + ClusterIP Service |
| `k8s/frontend-deployment.yaml` | Frontend Deployment (2 replicas) + ClusterIP Service |
| `k8s/mongodb-statefulset.yaml` | MongoDB StatefulSet (1 replica) + headless Service + 10Gi PVC |
| `k8s/ingress.yaml` | Ingress rules for `auth.example.com` and `app.example.com` |

### Step 1: Create Namespace

```bash
kubectl apply -f k8s/namespace.yaml
```

### Step 2: Create Secrets

```bash
# Create JWT key pair secret from files
kubectl create secret generic nyxid-jwt-keys \
  --namespace nyxid \
  --from-file=private.pem=keys/private.pem \
  --from-file=public.pem=keys/public.pem

# Create application secrets
kubectl create secret generic nyxid-secrets \
  --namespace nyxid \
  --from-literal=DATABASE_URL='mongodb://nyxid:YOUR_PASSWORD@nyxid-mongodb:27017/nyxid?authSource=admin' \
  --from-literal=ENCRYPTION_KEY="$(openssl rand -hex 32)"

# Create MongoDB credentials
kubectl create secret generic nyxid-mongo-secret \
  --namespace nyxid \
  --from-literal=MONGO_INITDB_ROOT_USERNAME=nyxid \
  --from-literal=MONGO_INITDB_ROOT_PASSWORD='YOUR_STRONG_PASSWORD'
```

### Step 3: Update Configuration

Edit `k8s/configmap.yaml` and replace the placeholder URLs:
- `BASE_URL`: Your backend's public URL (e.g., `https://auth.yourdomain.com`)
- `FRONTEND_URL`: Your frontend's public URL (e.g., `https://app.yourdomain.com`)
- `JWT_ISSUER`: Defaults to `BASE_URL`; leave unset unless you need a custom issuer

Edit `k8s/ingress.yaml` and replace `auth.example.com` / `app.example.com` with your domains.

### Step 4: Update Image References

Edit `k8s/backend-deployment.yaml` and `k8s/frontend-deployment.yaml` to reference your registry:

```yaml
image: registry.example.com/nyxid/backend:v0.1.0  # replace
image: registry.example.com/nyxid/frontend:v0.1.0  # replace
```

### Step 5: Apply All Manifests

```bash
kubectl apply -f k8s/
```

### Step 6: Verify

```bash
# Check all resources
kubectl get all -n nyxid

# Check backend health
kubectl port-forward -n nyxid svc/nyxid-backend 3001:3001
curl http://localhost:3001/health

# Check logs
kubectl logs -n nyxid -l app.kubernetes.io/name=nyxid-backend --tail=50
```

### Scaling

```bash
# Scale backend horizontally
kubectl scale deployment nyxid-backend -n nyxid --replicas=4

# Scale frontend
kubectl scale deployment nyxid-frontend -n nyxid --replicas=3
```

All backend instances must share the same RSA keys and encryption key (handled automatically via shared K8s secrets). See [Scaling](#scaling) for additional considerations.

### TLS with cert-manager

If using cert-manager for automatic TLS certificates:

1. Install cert-manager and create a `ClusterIssuer`
2. Uncomment the `cert-manager.io/cluster-issuer` annotation in `k8s/ingress.yaml`
3. cert-manager will automatically provision and renew certificates

---

## Environment Configuration

### Required Variables

| Variable         | Description                                    | Example                                         |
|------------------|------------------------------------------------|-------------------------------------------------|
| `DATABASE_URL`   | MongoDB connection string                      | `mongodb://user:pass@host:27017/nyxid?authSource=admin` |
| `ENCRYPTION_KEY` | 32-byte hex-encoded AES-256 key (64 hex chars) | Output of `openssl rand -hex 32`                |
| `ENCRYPTION_KEY_PREVIOUS` | Previous encryption key for key rotation (64 hex chars, optional) | Old `ENCRYPTION_KEY` value during rotation. Phase 1 supports one previous key at a time. |

### Server Variables

| Variable       | Default                 | Production Example               |
|----------------|-------------------------|----------------------------------|
| `PORT`         | `3001`                  | `3001`                           |
| `BASE_URL`     | `http://localhost:3001` | `https://auth.example.com`       |
| `FRONTEND_URL` | `http://localhost:3000` | `https://app.example.com`        |
| `ENVIRONMENT`  | `development`           | `production`                     |

### JWT Variables

| Variable               | Default            | Recommendation                    |
|------------------------|--------------------|-----------------------------------|
| `JWT_PRIVATE_KEY_PATH` | `keys/private.pem` | `/etc/nyxid/keys/private.pem`     |
| `JWT_PUBLIC_KEY_PATH`  | `keys/public.pem`  | `/etc/nyxid/keys/public.pem`      |
| `JWT_ISSUER`           | Same as `BASE_URL` | Leave unset (uses `BASE_URL`)     |
| `JWT_ACCESS_TTL_SECS`  | `900` (15 min)     | `900` or lower                    |
| `JWT_REFRESH_TTL_SECS` | `604800` (7 days)  | `604800` or lower                 |

### Rate Limiting

| Variable                | Default | Recommendation                            |
|-------------------------|---------|-------------------------------------------|
| `RATE_LIMIT_PER_SECOND` | `10`    | Adjust based on expected traffic          |
| `RATE_LIMIT_BURST`      | `30`    | Set higher for bursty workloads           |

### SMTP (Required for Email Features)

| Variable            | Description               | Example                     |
|---------------------|---------------------------|-----------------------------|
| `SMTP_HOST`         | SMTP server hostname      | `smtp.sendgrid.net`         |
| `SMTP_PORT`         | SMTP server port          | `587`                       |
| `SMTP_USERNAME`     | SMTP username             | `apikey`                    |
| `SMTP_PASSWORD`     | SMTP password             | `SG.xxxxx`                  |
| `SMTP_FROM_ADDRESS` | Sender email address      | `noreply@example.com`       |

### Social Login (Optional)

| Variable               | Description             |
|------------------------|-------------------------|
| `GOOGLE_CLIENT_ID`     | Google OAuth client ID  |
| `GOOGLE_CLIENT_SECRET` | Google OAuth secret     |
| `GITHUB_CLIENT_ID`     | GitHub OAuth client ID  |
| `GITHUB_CLIENT_SECRET` | GitHub OAuth secret     |

### Logging

| Variable   | Default                          | Production                       |
|------------|----------------------------------|----------------------------------|
| `RUST_LOG` | `nyxid=info,tower_http=info`     | `nyxid=info,tower_http=warn`     |

---

## Database Setup

### MongoDB Requirements

- Version 7.0 or higher (8.0 recommended)
- Authentication enabled
- TLS connections in production

### Connection String Format

```
mongodb://username:password@host:27017/nyxid?authSource=admin&tls=true
```

For replica sets:

```
mongodb://user:pass@host1:27017,host2:27017,host3:27017/nyxid?authSource=admin&replicaSet=rs0&tls=true
```

### Automatic Setup

NyxID creates all required collections and indexes automatically on first startup via `db::ensure_indexes()`. No manual migration steps are needed for fresh installations.

### Collections Created

NyxID uses 29 MongoDB collections, all created automatically at startup:

| Collection               | Purpose                                      |
|--------------------------|----------------------------------------------|
| `users`                  | User accounts (email, password hash, MFA status) |
| `sessions`               | Server-side sessions with hashed tokens      |
| `oauth_clients`          | Registered OAuth/OIDC clients (includes delegation_scopes) |
| `authorization_codes`    | Short-lived OIDC authorization codes         |
| `refresh_tokens`         | Refresh tokens with rotation chain tracking  |
| `api_keys`               | User-scoped API keys (hashed, with prefix)   |
| `downstream_services`    | Registered downstream services for proxying  |
| `user_service_connections` | Per-user connections and encrypted credentials |
| `mfa_factors`            | TOTP factors and encrypted recovery codes    |
| `service_endpoints`      | Registered API endpoints per service (MCP tools) |
| `provider_configs`       | External provider registry (encrypted OAuth creds) |
| `user_provider_tokens`   | Per-user encrypted provider tokens (API keys/OAuth) |
| `user_provider_credentials` | Per-user encrypted provider credentials    |
| `service_provider_requirements` | Provider token requirements per service |
| `oauth_states`           | Temporary OAuth state for provider flows     |
| `roles`                  | Role definitions with permissions and scoping |
| `groups`                 | Group definitions with role inheritance      |
| `consents`               | User OAuth consent records per client        |
| `service_accounts`       | Non-human (machine) identity definitions     |
| `service_account_tokens` | Issued service account JWT records for revocation |
| `approval_requests`      | Pending/resolved approval requests for proxy access |
| `approval_grants`        | Cached approval grants (time-limited, revocable) |
| `service_approval_configs` | Per-service approval overrides (per user)  |
| `notification_channels`  | Per-user notification preferences, Telegram links, push device tokens |
| `nodes`                  | Registered credential nodes (per user, with auth token hash and status) |
| `node_service_bindings`  | Service-to-node routing bindings             |
| `node_registration_tokens` | One-time tokens for node registration (TTL-indexed) |
| `mcp_sessions`           | MCP protocol session state                   |
| `audit_log`              | Immutable audit trail of security events     |

### MongoDB Atlas

For managed MongoDB (Atlas):

1. Create a cluster (M10+ for production)
2. Create a database user with `readWrite` role on the `nyxid` database
3. Whitelist your server IP addresses
4. Use the provided connection string with `tls=true`

---

## RSA Key Management

NyxID uses a 4096-bit RSA key pair for JWT signing (RS256).

### Generating Keys

```bash
# Generate private key
openssl genrsa -out keys/private.pem 4096

# Extract public key
openssl rsa -in keys/private.pem -pubout -out keys/public.pem

# Restrict permissions (private key should be read-only by the app)
chmod 600 keys/private.pem
chmod 644 keys/public.pem
```

### Development Mode

In development (`ENVIRONMENT=development`), NyxID auto-generates keys if the configured paths do not exist. This is disabled in production.

### Production Key Management

- Store keys outside the application directory (e.g., `/etc/nyxid/keys/`)
- Use filesystem permissions to restrict access (`chmod 600`)
- Mount keys as read-only volumes in Docker
- Rotate keys periodically (update both files, restart the server)
- Back up the private key securely -- losing it invalidates all issued JWTs

### Key Rotation

When rotating keys:

1. Generate a new key pair
2. Replace the key files
3. Restart the NyxID backend
4. All existing JWTs signed with the old key will fail verification
5. Users will need to re-authenticate (refresh tokens will also be invalidated)

To avoid downtime during rotation, implement a multi-key verification strategy at the application level (not yet supported -- planned for a future release).

---

## Reverse Proxy Configuration

NyxID should run behind a reverse proxy in production for TLS termination, load balancing, and `X-Forwarded-For` header injection.

### Caddy (Recommended)

```
auth.example.com {
    reverse_proxy localhost:3001

    header {
        X-Forwarded-For {remote_host}
    }
}

app.example.com {
    root * /var/www/nyxid/frontend/dist
    file_server

    # SPA fallback
    try_files {path} /index.html
}
```

Caddy handles TLS certificates automatically via Let's Encrypt.

### Nginx

```nginx
upstream nyxid_backend {
    server 127.0.0.1:3001;
}

server {
    listen 443 ssl http2;
    server_name auth.example.com;

    ssl_certificate     /etc/letsencrypt/live/auth.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/auth.example.com/privkey.pem;

    # Security headers (NyxID also sets these, but defense in depth)
    add_header X-Frame-Options DENY always;
    add_header X-Content-Type-Options nosniff always;

    # Proxy to backend
    location / {
        proxy_pass http://nyxid_backend;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # WebSocket support (if needed in future)
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }
}

# Frontend static files
server {
    listen 443 ssl http2;
    server_name app.example.com;

    ssl_certificate     /etc/letsencrypt/live/app.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/app.example.com/privkey.pem;

    root /var/www/nyxid/frontend/dist;
    index index.html;

    # SPA fallback
    location / {
        try_files $uri $uri/ /index.html;
    }

    # Cache static assets
    location ~* \.(js|css|png|jpg|jpeg|gif|ico|svg|woff2?)$ {
        expires 1y;
        add_header Cache-Control "public, immutable";
    }
}

# Redirect HTTP to HTTPS
server {
    listen 80;
    server_name auth.example.com app.example.com;
    return 301 https://$host$request_uri;
}
```

---

## TLS and Certificates

### Requirements

- TLS 1.2+ (TLS 1.3 preferred)
- Valid certificate from a trusted CA (Let's Encrypt, etc.)
- HSTS is enforced by NyxID's security headers middleware

### Let's Encrypt with Certbot

```bash
# Install certbot
sudo apt install certbot python3-certbot-nginx

# Obtain certificates
sudo certbot --nginx -d auth.example.com -d app.example.com

# Auto-renewal (cron)
sudo certbot renew --dry-run
```

### Cookie Security

NyxID automatically sets the `Secure` flag on authentication cookies when `BASE_URL` does not start with `http://localhost` or `http://127.0.0.1`. Ensure `BASE_URL` uses `https://` in production so that cookies are transmitted only over TLS.

---

## Frontend Deployment

### Static Hosting

The frontend build output (`frontend/dist/`) is a static SPA. Deploy it to any static hosting provider:

- **CDN**: Cloudflare Pages, Vercel, Netlify
- **Object Storage**: S3 + CloudFront, GCS + Cloud CDN
- **Self-hosted**: Nginx, Caddy, Apache

### SPA Routing

The frontend uses client-side routing. Configure your server to serve `index.html` for all routes that don't match a static file (see the Nginx and Caddy examples above).

### Environment at Build Time

The frontend API URL is configured in `frontend/src/lib/api-client.ts`. For production, update this to point to your backend's public URL before building:

```bash
# Build with production API URL
cd frontend
npm run build
```

### Cache Strategy

- HTML files: no-cache (always fetch latest)
- JS/CSS with content hashes: cache forever (`Cache-Control: public, max-age=31536000, immutable`)
- Vite automatically adds content hashes to built assets

---

## Health Checks and Monitoring

### Health Endpoint

```bash
curl https://auth.example.com/health
# {"status":"ok","version":"0.1.0"}
```

Use this for:
- Load balancer health checks
- Container orchestration liveness probes
- Uptime monitoring (Uptime Robot, Pingdom, etc.)

### Kubernetes Probes

```yaml
livenessProbe:
  httpGet:
    path: /health
    port: 3001
  initialDelaySeconds: 5
  periodSeconds: 10
readinessProbe:
  httpGet:
    path: /health
    port: 3001
  initialDelaySeconds: 5
  periodSeconds: 5
```

### Logging

NyxID uses structured logging via `tracing`. Configure log levels with `RUST_LOG`:

```bash
# Development (verbose)
RUST_LOG=nyxid=debug,tower_http=debug

# Production (essential only)
RUST_LOG=nyxid=info,tower_http=warn

# Debug specific modules
RUST_LOG=nyxid::services::auth_service=debug,nyxid=info
```

Logs are written to stdout in a human-readable format. For JSON-structured logs suitable for log aggregation (ELK, Datadog, etc.), modify the tracing subscriber in `main.rs` to use `.json()` format.

### Audit Log

NyxID maintains an internal audit log in the `audit_log` MongoDB collection. Query it via the admin API:

```bash
curl "https://auth.example.com/api/v1/admin/audit-log?page=1&per_page=50" \
  -H "Authorization: Bearer <admin_token>"
```

---

## Backup and Recovery

### MongoDB Backup

```bash
# Full backup using mongodump
mongodump --uri="mongodb://user:pass@host:27017/nyxid?authSource=admin" \
  --out=/backups/nyxid-$(date +%Y%m%d)

# Restore from backup
mongorestore --uri="mongodb://user:pass@host:27017/nyxid?authSource=admin" \
  /backups/nyxid-20260101/
```

### Automated Backup Schedule

```bash
# Cron job: daily backup at 2am, retain 30 days
0 2 * * * mongodump --uri="$DATABASE_URL" --out=/backups/nyxid-$(date +\%Y\%m\%d) && find /backups -maxdepth 1 -name "nyxid-*" -mtime +30 -exec rm -rf {} \;
```

### What to Back Up

| Item             | Location                 | Frequency |
|------------------|--------------------------|-----------|
| MongoDB database | Via `mongodump`          | Daily     |
| RSA key pair     | `keys/private.pem`       | Once (store securely) |
| Environment file | `.env.production`        | On change |

### Recovery Steps

1. Restore MongoDB from backup (`mongorestore`)
2. Place RSA keys at the configured paths
3. Restore the environment file
4. Start the NyxID backend -- indexes are recreated automatically
5. Verify with `curl /health`

---

## Scaling

### Horizontal Scaling (Multiple Backend Instances)

NyxID is stateless at the application layer. All state lives in MongoDB. You can run multiple instances behind a load balancer:

```
                    +---> NyxID instance 1 ---+
Load Balancer ----->+---> NyxID instance 2 ---+---> MongoDB
                    +---> NyxID instance 3 ---+
```

Requirements for horizontal scaling:
- All instances must share the same RSA key pair
- All instances must use the same `ENCRYPTION_KEY` and `ENCRYPTION_KEY_PREVIOUS` (if set)
- All instances must connect to the same MongoDB instance or replica set
- Load balancer should use sticky sessions or round-robin (both work since state is in the database)

### MongoDB Scaling

- **Replica Set**: For high availability (3+ nodes recommended)
- **Sharding**: Not required at typical auth workloads (millions of users are fine on a single replica set)
- **Read Preferences**: Use `secondaryPreferred` for read-heavy admin/audit queries

### Rate Limiting Caveat

The current rate limiter is per-instance (in-memory). When running multiple instances, each instance tracks its own counters independently. For distributed rate limiting, consider:
- An external rate limiter (e.g., Redis-backed)
- Rate limiting at the reverse proxy / load balancer level
- Accepting that per-instance limits are approximate but still effective

---

## Initial Admin Setup

NyxID ships without a default admin account. You must create the first admin using one of the two methods below.

### Option 1: Bootstrap Endpoint (Recommended for First Deploy)

When the database is empty (zero users), NyxID exposes a one-time setup endpoint:

```bash
curl -X POST http://localhost:3001/api/v1/auth/setup \
  -H "Content-Type: application/json" \
  -d '{
    "email": "admin@example.com",
    "password": "secureadminpassword123",
    "display_name": "Admin"
  }'
```

This creates a user with `is_admin = true` and `email_verified = true`. The endpoint automatically locks itself after the first user is created -- subsequent calls return `403 Forbidden`.

### Option 2: CLI Promote (For Existing Users)

If a user has already registered through the normal flow, promote them to admin via the command line:

```bash
# Using cargo
cargo run --manifest-path backend/Cargo.toml -- --promote-admin admin@example.com

# Using the built binary
./backend/target/release/nyxid --promote-admin admin@example.com
```

This sets `is_admin = true` and `email_verified = true` on the user. The command exits after completion (does not start the server).

### Verification

After creating the admin, verify by logging in and accessing an admin endpoint:

```bash
# Login as admin
curl -X POST http://localhost:3001/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -c cookies.txt \
  -d '{"email": "admin@example.com", "password": "secureadminpassword123"}'

# Access admin endpoint
curl http://localhost:3001/api/v1/admin/users \
  -b cookies.txt
```

---

## CLI Reference

NyxID supports command-line flags for administrative operations.

```
nyxid [OPTIONS]

Options:
    --promote-admin <EMAIL>    Promote an existing user to admin by email, then exit
    -h, --help                 Print help
    -V, --version              Print version
```

### --promote-admin

Promotes an existing user to admin by their email address. Requires a running MongoDB instance (reads `DATABASE_URL` from environment or `.env`).

```bash
nyxid --promote-admin user@example.com
```

Behavior:
- Finds the user by email (case-insensitive)
- Sets `is_admin = true` and `email_verified = true`
- Logs an `admin_promoted` audit event
- Prints the user ID on success
- Exits with code 0 on success, 1 on failure
- Does not start the HTTP server

Errors:
- `No user found with email: ...` -- The email is not registered
- `User ... is already an admin` -- The user already has admin privileges

---

## OIDC Provider Configuration

NyxID acts as a full OpenID Connect (OIDC) identity provider. Downstream services can delegate authentication to NyxID using the standard Authorization Code flow with PKCE.

### How It Works

1. An admin creates a downstream service in NyxID with `auth_type: "oidc"`.
2. NyxID auto-provisions an OAuth client and generates a `client_id` and `client_secret`.
3. The downstream service uses the OIDC discovery endpoint to auto-configure itself.
4. Users authenticate via NyxID and are redirected back to the downstream service with an authorization code.
5. The downstream service exchanges the code for access, refresh, and ID tokens.

### Step 1: Create an OIDC Service

```bash
curl -X POST https://auth.example.com/api/v1/services \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Customer Portal",
    "base_url": "https://portal.example.com",
    "auth_type": "oidc"
  }'
```

NyxID returns the service with an `oauth_client_id`. The default redirect URI is set to `https://portal.example.com/callback`.

### Step 2: Retrieve OIDC Credentials

```bash
curl https://auth.example.com/api/v1/services/<service_id>/oidc-credentials \
  -H "Authorization: Bearer <admin_access_token>"
```

Response includes:
- `client_id` -- The OAuth client identifier
- `client_secret` -- The client secret (store securely)
- `redirect_uris` -- Registered callback URLs
- `issuer`, `authorization_endpoint`, `token_endpoint`, `userinfo_endpoint`, `jwks_uri` -- OIDC endpoints

### Step 3: Configure Redirect URIs (Optional)

If the default `{base_url}/callback` is not correct, update the redirect URIs:

```bash
curl -X PUT https://auth.example.com/api/v1/services/<service_id>/redirect-uris \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "redirect_uris": [
      "https://portal.example.com/auth/callback",
      "https://portal.example.com/api/auth/callback"
    ]
  }'
```

### Step 4: Configure the Downstream Service

The downstream service uses the OIDC discovery URL to auto-configure the authorization flow. The discovery document is available at:

```
https://auth.example.com/.well-known/openid-configuration
```

### OIDC Protocol Details

| Parameter            | Value                                                |
|----------------------|------------------------------------------------------|
| Flow                 | Authorization Code with PKCE (S256)                  |
| Response Type        | `code`                                               |
| Grant Types          | `authorization_code`, `refresh_token`                |
| Token Signing        | RS256 (RSA SHA-256, 4096-bit key)                    |
| Scopes               | `openid`, `profile`, `email`                         |
| Client Auth Methods  | `client_secret_post`, `none` (for public clients)    |
| ID Token Claims      | `sub`, `iss`, `aud`, `exp`, `iat`, `email`, `email_verified`, `name`, `picture`, `nonce` |

### Example: NextAuth.js Configuration

```javascript
// pages/api/auth/[...nextauth].js
import NextAuth from "next-auth";

export default NextAuth({
  providers: [
    {
      id: "nyxid",
      name: "NyxID",
      type: "oidc",
      issuer: "https://auth.example.com",
      clientId: "<client_id>",
      clientSecret: "<client_secret>",
      checks: ["pkce", "state"],
    },
  ],
});
```

NextAuth.js will automatically fetch the discovery document from `https://auth.example.com/.well-known/openid-configuration` and configure all endpoints.

### Example: Passport.js Configuration

```javascript
// auth.js
const { Strategy } = require("openid-client");
const { Issuer } = require("openid-client");

async function setupAuth(app) {
  const nyxidIssuer = await Issuer.discover("https://auth.example.com");

  const client = new nyxidIssuer.Client({
    client_id: "<client_id>",
    client_secret: "<client_secret>",
    redirect_uris: ["https://portal.example.com/callback"],
    response_types: ["code"],
  });

  app.use(
    "/auth",
    new Strategy({ client, usePKCE: true }, (tokenSet, userinfo, done) => {
      done(null, userinfo);
    })
  );
}
```

### Secret Rotation

To rotate an OIDC client secret without downtime:

1. Regenerate the secret via the API:
   ```bash
   curl -X POST https://auth.example.com/api/v1/services/<service_id>/regenerate-secret \
     -H "Authorization: Bearer <admin_access_token>"
   ```
2. Update the downstream service configuration with the new `client_secret`.
3. Deploy the downstream service. The old secret is immediately invalidated.

---

## MCP Proxy Deployment

The MCP proxy exposes NyxID-managed downstream services as Model Context Protocol (MCP) tools, allowing AI assistants to interact with registered APIs through a single authenticated endpoint.

### Architecture

```
AI Client --[MCP/HTTP]--> MCP Proxy --[OAuth 2.1]--> NyxID Backend --[proxy]--> Downstream APIs
```

The proxy authenticates users via NyxID's OIDC endpoints and dynamically generates MCP tools from the service endpoints registered in NyxID.

### Configuration

| Variable | Description | Example |
|---|---|---|
| `NYXID_URL` | NyxID backend URL | `http://backend:3001` (Docker) or `https://auth.example.com` |
| `NYXID_CLIENT_ID` | OAuth client ID registered in NyxID | `mcp-proxy` |
| `NYXID_CLIENT_SECRET` | OAuth client secret | (from NyxID admin panel) |
| `MCP_PORT` | Port for the MCP HTTP server | `3002` |

### Setup

1. Register an OAuth client in NyxID for the MCP proxy (type: confidential, scopes: `openid profile email`).
2. Configure the environment variables (see above).
3. Start the proxy:

```bash
# Development
cd mcp-proxy
cp .env.example .env
# Edit .env with your NyxID credentials
npm install
npm run dev

# Production (Docker)
docker compose -f docker-compose.prod.yml up -d mcp-proxy
```

### MCP Client Configuration

MCP clients connect to the proxy's Streamable HTTP endpoint:

```
POST http://localhost:3002/mcp
```

The proxy serves OAuth 2.1 Protected Resource Metadata at:

```
GET http://localhost:3002/.well-known/oauth-protected-resource
```

MCP clients that support OAuth (e.g., Claude Desktop) will automatically discover NyxID as the authorization server and prompt the user to authenticate via the browser.

### Lazy Tool Loading

To optimize performance, NyxID implements dynamic tool loading:

**Session Initialization:**
- MCP sessions start with only 3 meta-tools:
  - `nyx__search_tools` -- Search and activate service tools by keyword
  - `nyx__discover_services` -- Browse available services (does not activate)
  - `nyx__connect_service` -- Connect to a specific service and activate its tools

**On-Demand Activation:**
- When the LLM calls `nyx__search_tools`, the server activates matching service tools and sends a `notifications/tools/list_changed` notification
- When the LLM calls `nyx__connect_service`, that service's tools are activated
- Clients (Cursor, Claude Code) automatically refresh their tool list when they receive the notification

**Constraints:**
- Maximum 20 activated services per session (bounded memory usage)
- `nyx__discover_services` is browse-only and does NOT activate tools

### How Tools Are Generated

For each active downstream service with registered endpoints, the proxy dynamically creates MCP tools following this pattern:

- **Tool name**: `{service_slug}__{endpoint_name}` (e.g., `stripe__list_customers`)
- **Description**: `[{service_name}] {endpoint_description}`
- **Input schema**: Derived from endpoint parameters and request body schema

Only services with valid connections and satisfied credentials are included:
- `connection` services require the user to have stored an encrypted per-user credential
- `internal` services only require an active connection record
- `provider` services are excluded (not proxyable)

### Backward Compatibility

The REST endpoint `/api/v1/mcp/config` still returns the full list of all tools for clients that don't support the MCP protocol.

When a tool is called, the proxy:
1. Resolves the service and endpoint from the tool name.
2. Substitutes path parameters from tool arguments (URL-encoded for safety).
3. Separates remaining arguments into query parameters and request body.
4. Forwards the request through NyxID's authenticated proxy (`/api/v1/proxy/{service_id}/{path}`).

### Client Configuration: Claude Code

Add the NyxID MCP server to your Claude Code settings (`~/.claude/settings.json` or project `.mcp.json`):

```json
{
  "mcpServers": {
    "nyxid": {
      "url": "https://mcp.example.com/mcp"
    }
  }
}
```

Claude Code supports OAuth-based MCP servers natively. On first use, it will open a browser window for authentication via NyxID.

### Client Configuration: Cursor

Add the NyxID MCP server to your Cursor settings (`.cursor/mcp.json`):

```json
{
  "mcpServers": {
    "nyxid": {
      "url": "https://mcp.example.com/mcp"
    }
  }
}
```

Cursor will detect the OAuth Protected Resource Metadata at `/.well-known/oauth-protected-resource` and prompt for authentication automatically.

### Health Check

```bash
curl http://localhost:3002/health
# {"status":"ok","sessions":0}
```

### Reverse Proxy

If exposing the MCP proxy externally, add it to your reverse proxy configuration:

```
# Caddy
mcp.example.com {
    reverse_proxy localhost:3002
}
```

```nginx
# Nginx
server {
    listen 443 ssl http2;
    server_name mcp.example.com;

    location / {
        proxy_pass http://127.0.0.1:3002;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_http_version 1.1;
        proxy_set_header Connection "";
        proxy_read_timeout 300s;
    }
}
```

Note the extended `proxy_read_timeout` -- MCP tool calls that proxy to slow downstream APIs may take longer than the default timeout.

---

## Credential Broker Setup

NyxID's credential broker feature lets users store encrypted API keys and OAuth tokens for external providers (e.g., OpenAI, Anthropic, Google AI). These tokens are injected into proxy requests on behalf of users.

### Prerequisites

The credential broker reuses the existing `ENCRYPTION_KEY` and `BASE_URL` environment variables. No additional configuration is needed.

### Step 1: Configure Providers

After the initial admin setup, register provider configurations via the API:

**API key provider (e.g., OpenAI):**

```bash
curl -X POST https://auth.example.com/api/v1/providers \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "OpenAI",
    "slug": "openai",
    "provider_type": "api_key",
    "description": "OpenAI API for GPT models",
    "api_key_instructions": "Visit https://platform.openai.com/api-keys to create a key",
    "api_key_url": "https://platform.openai.com/api-keys",
    "icon_url": "https://example.com/icons/openai.svg",
    "documentation_url": "https://platform.openai.com/docs"
  }'
```

**OAuth2 provider (e.g., Google AI):**

```bash
curl -X POST https://auth.example.com/api/v1/providers \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Google AI",
    "slug": "google-ai",
    "provider_type": "oauth2",
    "authorization_url": "https://accounts.google.com/o/oauth2/v2/auth",
    "token_url": "https://oauth2.googleapis.com/token",
    "revocation_url": "https://oauth2.googleapis.com/revoke",
    "default_scopes": ["https://www.googleapis.com/auth/generative-language"],
    "client_id": "your-google-client-id",
    "client_secret": "your-google-client-secret",
    "supports_pkce": true
  }'
```

### Step 2: Configure Service Requirements

For each downstream service that needs provider tokens, add provider requirements:

```bash
curl -X POST https://auth.example.com/api/v1/services/<service_id>/requirements \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "provider_config_id": "<openai_provider_id>",
    "required": true,
    "injection_method": "bearer"
  }'
```

### Step 3: Configure Identity Propagation (Optional)

Enable identity propagation on downstream services to forward user identity:

```bash
curl -X PUT https://auth.example.com/api/v1/services/<service_id> \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "identity_propagation_mode": "both",
    "identity_include_user_id": true,
    "identity_include_email": true,
    "identity_include_name": true,
    "identity_jwt_audience": "https://my-service.example.com"
  }'
```

### Common Provider Configurations

| Provider   | Type    | Auth Model  | Notes                                    |
|------------|---------|-------------|------------------------------------------|
| OpenAI     | api_key | API key     | `Authorization: Bearer <key>` injection  |
| Anthropic  | api_key | API key     | `x-api-key: <key>` header injection      |
| Mistral    | api_key | API key     | `Authorization: Bearer <key>` injection  |
| Cohere     | api_key | API key     | `Authorization: Bearer <key>` injection  |
| Google AI  | oauth2  | OAuth2+PKCE | Requires Google Cloud OAuth credentials  |
| Azure OpenAI | api_key | API key  | `api-key: <key>` header injection        |

### OAuth2 Redirect URI

For OAuth2 providers, register the following redirect URI in the provider's developer console:

```
https://auth.example.com/api/v1/providers/callback
```

This is the generic callback endpoint that handles all provider OAuth flows.

---

## Telegram Bot Setup

The approval system uses a Telegram bot to send push notifications for access approval requests. This is optional -- the approval system works via the web UI without Telegram.

### 1. Create a Telegram Bot

1. Open Telegram and search for [@BotFather](https://t.me/BotFather)
2. Send `/newbot` and follow the prompts to name your bot
3. BotFather will give you a **bot token** (e.g. `123456789:ABCdef...`)
4. Note the **bot username** (e.g. `NyxIDBot`)

### 2. Configure Environment Variables

Add to your `.env` or environment:

```bash
TELEGRAM_BOT_TOKEN=123456789:ABCdefGHIjklMNO     # From BotFather
TELEGRAM_BOT_USERNAME=NyxIDBot                     # Without @
```

For **production (webhook mode)**, also set:

```bash
TELEGRAM_WEBHOOK_SECRET=your-random-secret-string  # Generate with: openssl rand -hex 32
TELEGRAM_WEBHOOK_URL=https://auth.nyxid.dev/api/v1/webhooks/telegram
```

The `TELEGRAM_WEBHOOK_URL` must be a publicly accessible HTTPS URL that Telegram can reach.

For **development (long polling mode)**, omit `TELEGRAM_WEBHOOK_URL` and `TELEGRAM_WEBHOOK_SECRET`. The backend will automatically fall back to `getUpdates` long polling -- no public URL or tunnel required.

### 3. Delivery Modes

NyxID supports two Telegram delivery modes, selected automatically based on environment configuration:

| Mode | When | How |
|------|------|-----|
| **Webhook** (production) | `TELEGRAM_WEBHOOK_URL` + `TELEGRAM_WEBHOOK_SECRET` are set | Backend calls `setWebhook` at startup. Telegram pushes updates to `POST /api/v1/webhooks/telegram`. |
| **Long polling** (development) | Only `TELEGRAM_BOT_TOKEN` is set | Backend calls `deleteWebhook` at startup, then polls `getUpdates` in a background task (30s timeout). |

Both modes share the same update processing logic and support all features (approval callbacks, account linking).

#### Webhook Mode Setup

Register the webhook URL with Telegram using the Bot API:

```bash
curl -X POST "https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/setWebhook" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://auth.nyxid.dev/api/v1/webhooks/telegram",
    "secret_token": "your-random-secret-string",
    "allowed_updates": ["callback_query", "message"]
  }'
```

Verify the webhook is set:

```bash
curl "https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/getWebhookInfo"
```

> **Note:** The backend also registers the webhook at startup when the env vars are set. The manual curl command is useful for verification or re-registration.

#### Long Polling Mode Setup

No additional setup needed. Just set `TELEGRAM_BOT_TOKEN` (and optionally `TELEGRAM_BOT_USERNAME`) and start the backend. The polling loop starts automatically and logs:

```
INFO nyxid: Telegram polling mode: starting getUpdates loop
```

If a webhook was previously registered, the backend calls `deleteWebhook` first to avoid conflicts.

### 4. User Account Linking

Users link their Telegram account from the NyxID settings page:

1. User navigates to **Settings > Notifications** in the NyxID dashboard
2. Clicks **Link Telegram** to generate a one-time link code
3. Sends `/start NYXID-XXXXXXXX` to the bot in Telegram
4. The bot confirms the link and starts sending approval notifications

### 5. Development Setup

For local development, **long polling mode** is recommended (see above). Simply set `TELEGRAM_BOT_TOKEN` and omit the webhook variables.

If you prefer webhook mode locally, you need a public URL:

- **ngrok**: `ngrok http 3001` and use the generated HTTPS URL
- **Cloudflare Tunnel**: `cloudflared tunnel --url http://localhost:3001`

Set `TELEGRAM_WEBHOOK_URL` to the public URL + `/api/v1/webhooks/telegram`.

### 6. Approval Expiry Configuration

The background task that expires timed-out approval requests runs every `APPROVAL_EXPIRY_INTERVAL_SECS` seconds (default: 5). Adjust this if you need more or less frequent expiry checks:

```bash
APPROVAL_EXPIRY_INTERVAL_SECS=10  # Check every 10 seconds
```

---

## Security Hardening Checklist

### Before Going Live

- [ ] `ENVIRONMENT` is set to `production`
- [ ] `ENCRYPTION_KEY` is a unique, randomly generated 32-byte key (not the `.env.example` placeholder)
- [ ] RSA key pair is pre-generated and mounted read-only (not auto-generated)
- [ ] `BASE_URL` uses `https://` (enables Secure cookie flag)
- [ ] `FRONTEND_URL` uses `https://` (CORS origin)
- [ ] `DATABASE_URL` uses TLS (`?tls=true`)
- [ ] MongoDB authentication is enabled with a strong password
- [ ] If using Telegram approval in webhook mode: `TELEGRAM_WEBHOOK_SECRET` is a randomly generated string
- [ ] If using Telegram approval in webhook mode: `TELEGRAM_WEBHOOK_URL` uses `https://`
- [ ] If using Telegram approval in polling mode (dev only): `TELEGRAM_WEBHOOK_URL` is not set
- [ ] SMTP is configured for transactional email (verify-email, reset-password)
- [ ] Social login secrets are set (if using social login)
- [ ] TLS is terminated at the reverse proxy
- [ ] Reverse proxy sets `X-Forwarded-For` for accurate IP-based rate limiting
- [ ] `RUST_LOG` is set to `nyxid=info,tower_http=warn` (no debug output)
- [ ] Private key file permissions are `600`
- [ ] `.env` file is not accessible via the web server
- [ ] No development tools (Mailpit, debug endpoints) are exposed
- [ ] Firewall rules restrict MongoDB port to backend servers only
- [ ] Backup strategy is in place and tested

### Ongoing

- [ ] Monitor the `/health` endpoint
- [ ] Review audit logs periodically
- [ ] Rotate the `ENCRYPTION_KEY` deliberately, not on an unattended schedule, unless you have completed re-encryption of old-key data and `/health` shows zero fallback decrypts for the old key (see [SECURITY.md](SECURITY.md#key-rotation))
- [ ] Rotate RSA keys on a schedule
- [ ] Keep dependencies updated (`cargo update`, `npm update`)
- [ ] Subscribe to security advisories for critical dependencies (argon2, jsonwebtoken, rsa, aes-gcm)

---

## Troubleshooting

### Backend fails to start

**"ENCRYPTION_KEY must be set"**
- Ensure `ENCRYPTION_KEY` is defined in your `.env` or environment variables.

**"ENCRYPTION_KEY is all zeros"**
- Replace the placeholder key from `.env.example` with a real key: `openssl rand -hex 32`

**"Failed to connect to database"**
- Verify MongoDB is running: `mongosh --eval "db.adminCommand('ping')"`
- Check `DATABASE_URL` format and credentials
- Ensure network connectivity between the backend and MongoDB

**"Failed to load JWT keys"**
- In production, RSA keys must exist at the configured paths
- Check file permissions: `ls -la keys/`
- Verify the key format: `openssl rsa -in keys/private.pem -check`

### Authentication issues

**Cookies not being set**
- Ensure `BASE_URL` and `FRONTEND_URL` match your actual URLs
- Check CORS: the frontend origin must exactly match `FRONTEND_URL`
- In production, cookies require `Secure` flag (HTTPS only)

**JWT verification fails after key rotation**
- This is expected. All tokens signed with the old key are invalidated.
- Users need to re-authenticate.

**Rate limiting too aggressive**
- Increase `RATE_LIMIT_PER_SECOND` and `RATE_LIMIT_BURST`
- Check if multiple instances are behind a load balancer (rate limits are per-instance)

### Database issues

**Slow queries**
- NyxID creates indexes automatically. Verify they exist: `db.users.getIndexes()`
- Check MongoDB logs for slow query warnings
- Consider adding the MongoDB `slowms` profiler

**Connection pool exhaustion**
- Increase `DATABASE_MAX_CONNECTIONS` (default: 10)
- Check for connection leaks in custom extensions

### Frontend issues

**Blank page after deployment**
- Ensure SPA fallback is configured (serve `index.html` for all routes)
- Check browser console for CORS errors
- Verify the API URL in the frontend build matches the backend URL
