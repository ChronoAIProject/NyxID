# ── Stage 1: Build ────────────────────────────────────────────────────────────
FROM node:20-alpine AS builder

WORKDIR /build

COPY package.json package-lock.json ./
RUN npm ci

COPY . .

# Telemetry build args (see docs/TELEMETRY_M1.md §3). Empty by default,
# which means telemetry initializes to hard-off mode — behavior is
# byte-identical to a deploy without these vars. Supply them via
# `docker build --build-arg VITE_TELEMETRY_DSN=phc_…` on deploys where
# telemetry should be live.
ARG VITE_TELEMETRY_DSN=""
ARG VITE_TELEMETRY_HOST=""
ARG VITE_NYXID_SHARE_ANALYTICS="false"
ARG VITE_BUILD_HASH="dev"
ENV VITE_TELEMETRY_DSN=${VITE_TELEMETRY_DSN}
ENV VITE_TELEMETRY_HOST=${VITE_TELEMETRY_HOST}
ENV VITE_NYXID_SHARE_ANALYTICS=${VITE_NYXID_SHARE_ANALYTICS}
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
