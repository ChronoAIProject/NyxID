# ── Stage 1: Build ────────────────────────────────────────────────────────────
FROM node:20-alpine AS builder

WORKDIR /build

COPY package.json package-lock.json ./
RUN npm ci

COPY . .
RUN npm run build

# ── Stage 2: Runtime ─────────────────────────────────────────────────────────
FROM nginxinc/nginx-unprivileged:1.27-alpine

# Copy the built SPA
COPY --from=builder /build/dist /usr/share/nginx/html

# Copy the nginx config template (envsubst runs automatically via nginx entrypoint)
COPY nginx.conf.template /etc/nginx/templates/default.conf.template

ENV BACKEND_URL=http://backend:3001

EXPOSE 8080
