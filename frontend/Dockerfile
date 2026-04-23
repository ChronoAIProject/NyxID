# ── Stage 1: Build ────────────────────────────────────────────────────────────
FROM node:20-alpine AS builder

WORKDIR /build

COPY package.json package-lock.json ./
RUN npm ci

COPY . .

# Build hash is baked in for surface-version traceability (shows up in
# `X-NyxID-Client-Version` on requests). Non-secret, only used for
# debugging/observability. Telemetry DSN + host are NOT baked here —
# they are fetched at runtime from the backend's /api/v1/public/config
# endpoint so rotation does not require an image rebuild.
ARG VITE_BUILD_HASH="dev"
ENV VITE_BUILD_HASH=${VITE_BUILD_HASH}

RUN npm run build

# ── Stage 2: Runtime ─────────────────────────────────────────────────────────
FROM nginxinc/nginx-unprivileged:1.27-alpine

# Copy the built SPA
COPY --from=builder /build/dist /usr/share/nginx/html

# Copy the nginx config template (envsubst runs automatically via nginx entrypoint)
COPY nginx.conf.template /etc/nginx/templates/default.conf.template

ENV BACKEND_URL=http://backend:3001

EXPOSE 8080
