# ── Stage 1: Build ────────────────────────────────────────────────────────────
# NOTE: this image is built with the *repo root* as the Docker context
# (see .github/workflows/publish-images.yml), not the frontend/ dir. The
# frontend's vite `nyxid-docs-sync` plugin reads the public docs source at
# `../docs/site` (repo root) during `vite build` and its publish-gate fails
# the build if that tree is absent — so the docs must be present alongside
# the frontend at build time, which a frontend/-only context cannot provide.
FROM node:22.23.2-alpine@sha256:b6f26b36c8ff49624cfdac716b8ea1138d606df02586a77d364bb5536a634f85 AS builder

COPY security/npm-tools/package.json security/npm-tools/package-lock.json security/npm-tools/.npmrc /opt/nyxid-build-tools/
RUN npm ci --ignore-scripts --prefix /opt/nyxid-build-tools
ENV PATH=/opt/nyxid-build-tools/node_modules/.bin:$PATH
RUN test "$(npm --version)" = "11.21.1"

WORKDIR /build/frontend

COPY frontend/package.json frontend/package-lock.json frontend/.npmrc ./
RUN npm ci

COPY frontend/ ./

# Public docs site source. The docs-sync plugin resolves it relative to the
# frontend dir (`../docs/site`), so it must land at /build/docs/site.
COPY docs/ /build/docs/
COPY cli/resources/machine-container/seccomp.json /build/cli/resources/machine-container/seccomp.json

# Telemetry config (DSN, host, share-back flag) is fetched at runtime
# from the backend's /api/v1/public/config endpoint — no build-time
# env vars needed for telemetry. Rotation = edit .env.production,
# restart backend, done.

ARG NYXID_GIT_HASH
ARG SOURCE_DATE_EPOCH
ENV SOURCE_COMMIT=${NYXID_GIT_HASH}
ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

RUN npm run build

# ── Stage 2: Runtime ─────────────────────────────────────────────────────────
FROM nginxinc/nginx-unprivileged:1.30-alpine@sha256:782bb89768cc87b7f95c18b95baa1af250e5d451f26794f59363540161147c01

# Copy the built SPA
# Remove the base image's stock index/50x pages before copying shipped assets.
USER root
RUN rm -rf /usr/share/nginx/html/*
COPY --from=builder /build/frontend/dist /usr/share/nginx/html
COPY --from=builder /build/frontend/dist/legal-cache.conf /etc/nginx/legal-cache.conf

# The renderer validates origin-only inputs and emits complete nginx
# policies. No environment value is substituted as an nginx/header fragment.
USER root
RUN apk upgrade --no-cache \
    && apk add --no-cache python3 \
    && mkdir -p /etc/nginx/nyxid/generated \
    && chown 101:101 /etc/nginx/nyxid/generated
COPY frontend/nginx.conf.template /etc/nginx/nyxid/default.conf.template
COPY frontend/scripts/render-nginx.py /etc/nginx/nyxid/render-nginx.py
COPY --chmod=755 frontend/scripts/nginx-policy-entrypoint.sh /docker-entrypoint.d/40-nyxid-policy.sh
COPY frontend/nginx-default.conf /etc/nginx/conf.d/default.conf
USER 101

ENV BACKEND_URL=http://backend:3001
ENV NYXID_PUBLIC_ORIGIN=http://localhost:8080

EXPOSE 8080
