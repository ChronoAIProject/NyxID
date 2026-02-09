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

### Backend Dockerfile

Create `backend/Dockerfile`:

```dockerfile
# Build stage
FROM rust:1.85-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY backend/ backend/
RUN cargo build --release --manifest-path backend/Cargo.toml

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/backend/target/release/nyxid /usr/local/bin/nyxid
EXPOSE 3001
CMD ["nyxid"]
```

### Frontend Dockerfile

Create `frontend/Dockerfile`:

```dockerfile
FROM node:20-alpine AS builder
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci
COPY . .
RUN npm run build

FROM nginx:alpine
COPY --from=builder /app/dist /usr/share/nginx/html
COPY nginx.conf /etc/nginx/conf.d/default.conf
EXPOSE 80
```

### Production Docker Compose

Create `docker-compose.prod.yml`:

```yaml
services:
  backend:
    build:
      context: .
      dockerfile: backend/Dockerfile
    restart: unless-stopped
    ports:
      - "127.0.0.1:3001:3001"
    env_file:
      - .env.production
    volumes:
      - ./keys:/app/keys:ro
    depends_on:
      mongodb:
        condition: service_healthy
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:3001/health"]
      interval: 10s
      timeout: 5s
      retries: 3

  frontend:
    build:
      context: frontend
      dockerfile: Dockerfile
    restart: unless-stopped
    ports:
      - "127.0.0.1:3000:80"

  mongodb:
    image: mongo:8.0
    restart: unless-stopped
    environment:
      MONGO_INITDB_ROOT_USERNAME: ${MONGO_ROOT_USER}
      MONGO_INITDB_ROOT_PASSWORD: ${MONGO_ROOT_PASSWORD}
      MONGO_INITDB_DATABASE: nyxid
    volumes:
      - mongodb_data:/data/db
    healthcheck:
      test: ["CMD", "mongosh", "--eval", "db.adminCommand('ping')"]
      interval: 10s
      timeout: 5s
      retries: 5

volumes:
  mongodb_data:
```

### Running

```bash
docker compose -f docker-compose.prod.yml up -d
```

---

## Environment Configuration

### Required Variables

| Variable         | Description                                    | Example                                         |
|------------------|------------------------------------------------|-------------------------------------------------|
| `DATABASE_URL`   | MongoDB connection string                      | `mongodb://user:pass@host:27017/nyxid?authSource=admin` |
| `ENCRYPTION_KEY` | 32-byte hex-encoded AES-256 key (64 hex chars) | Output of `openssl rand -hex 32`                |

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
| `JWT_ISSUER`           | `nyxid`            | `auth.example.com`                |
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

NyxID creates all required collections and indexes automatically on first startup via `db::ensure_indexes()`. No manual migration steps are needed.

### Collections Created

| Collection               | Purpose                                      |
|--------------------------|----------------------------------------------|
| `users`                  | User accounts                                |
| `sessions`               | Server-side sessions                         |
| `oauth_clients`          | Registered OAuth/OIDC clients                |
| `authorization_codes`    | Short-lived OIDC authorization codes         |
| `refresh_tokens`         | Refresh tokens with rotation tracking        |
| `api_keys`               | User-scoped API keys                         |
| `downstream_services`    | Registered downstream services               |
| `user_service_connections` | Per-user credential overrides              |
| `mfa_factors`            | TOTP factors and recovery codes              |
| `audit_log`              | Immutable audit trail                        |

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
- All instances must use the same `ENCRYPTION_KEY`
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

## Security Hardening Checklist

### Before Going Live

- [ ] `ENVIRONMENT` is set to `production`
- [ ] `ENCRYPTION_KEY` is a unique, randomly generated 32-byte key (not the `.env.example` placeholder)
- [ ] RSA key pair is pre-generated and mounted read-only (not auto-generated)
- [ ] `BASE_URL` uses `https://` (enables Secure cookie flag)
- [ ] `FRONTEND_URL` uses `https://` (CORS origin)
- [ ] `DATABASE_URL` uses TLS (`?tls=true`)
- [ ] MongoDB authentication is enabled with a strong password
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
- [ ] Rotate the `ENCRYPTION_KEY` and RSA keys on a schedule
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
