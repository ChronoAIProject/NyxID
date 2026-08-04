# ── Stage 1: Build ────────────────────────────────────────────────────────────
# NOTE: this image is built with the *repo root* as the Docker context
# (see .github/workflows/publish-images.yml), not the frontend/ dir. The
# frontend's vite `nyxid-docs-sync` plugin reads the public docs source at
# `../docs/site` (repo root) during `vite build` and its publish-gate fails
# the build if that tree is absent — so the docs must be present alongside
# the frontend at build time, which a frontend/-only context cannot provide.
FROM node:20-alpine AS builder

WORKDIR /build/frontend

COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci

COPY frontend/ ./

# Public docs site source. The docs-sync plugin resolves it relative to the
# frontend dir (`../docs/site`), so it must land at /build/docs/site.
COPY docs/ /build/docs/

# Telemetry config (DSN, host, share-back flag) is fetched at runtime
# from the backend's /api/v1/public/config endpoint — no build-time
# env vars needed for telemetry. Rotation = edit .env.production,
# restart backend, done.

RUN npm run build

# ── Stage 2: Runtime ─────────────────────────────────────────────────────────
FROM nginxinc/nginx-unprivileged:1.27-alpine

# Copy the built SPA
COPY --from=builder /build/frontend/dist /usr/share/nginx/html

# Copy the nginx config template (envsubst runs automatically via nginx entrypoint)
COPY frontend/nginx.conf.template /etc/nginx/templates/default.conf.template

ENV BACKEND_URL=http://backend:3001

EXPOSE 8080
