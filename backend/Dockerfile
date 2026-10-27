# Normalize release-only workspace versions outside the dependency layer chain.
# COPY from this stage is keyed by its output, not the original manifest bytes.
FROM python:3.12-slim-bookworm AS dependency-recipe
WORKDIR /input

# Copy workspace manifests and lock file for dependency caching.
# `cloud-auth/` (NyxID#716) and `nyxid-crypto/` (NyxID#773) are workspace
# members; cargo refuses to resolve the workspace if any member's manifest
# is missing, so they must be staged here even though the backend binary
# only links cloud-auth directly via path (nyxid-crypto is CLI-only).
COPY Cargo.toml Cargo.lock ./
COPY backend/Cargo.toml backend/Cargo.toml
COPY cli/Cargo.toml cli/Cargo.toml
COPY cloud-auth/Cargo.toml cloud-auth/Cargo.toml
COPY service-adapters/Cargo.toml service-adapters/Cargo.toml
COPY mcp-demo/Cargo.toml mcp-demo/Cargo.toml
COPY nyxid-crypto/Cargo.toml nyxid-crypto/Cargo.toml
COPY machine/Cargo.toml machine/Cargo.toml
COPY permissions/Cargo.toml permissions/Cargo.toml
COPY machine-updater/Cargo.toml machine-updater/Cargo.toml
COPY scripts/docker-dependency-recipe.py /recipe.py
RUN python /recipe.py /input /recipe

# Build only dependencies; usable as --target dependencies for cache checks.
FROM rust:1.93-bookworm AS dependencies
WORKDIR /build

# Keep native amd64/arm64 builds bounded while allowing two compiler jobs.
# CI prepares 24 GiB total swap for the large server crate. Small builders can
# override this to 1; Cargo's default would otherwise use every available CPU.
ARG CARGO_BUILD_JOBS=2
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}
ENV CARGO_INCREMENTAL=0
COPY --from=dependency-recipe /recipe/ ./

# Create dummy entry points so cargo can resolve and compile dependencies.
# backend, cli, and mcp-demo are binary crates (need main.rs).
# cloud-auth and nyxid-crypto are library crates (need lib.rs); a stub
# `pub fn _stub(){}` is enough to satisfy cargo's check that the lib target
# exists. nyxid-crypto isn't in the backend binary's graph, so its stub is
# never compiled -- it only needs to exist for workspace resolution.
RUN mkdir -p backend/src && echo "fn main() {}" > backend/src/main.rs \
    && mkdir -p cli/src && echo "fn main() {}" > cli/src/main.rs \
    && mkdir -p cloud-auth/src && echo "pub fn _stub() {}" > cloud-auth/src/lib.rs \
    && mkdir -p service-adapters/src && echo "pub fn _stub() {}" > service-adapters/src/lib.rs \
    && mkdir -p mcp-demo/src && echo "fn main() {}" > mcp-demo/src/main.rs \
    && mkdir -p nyxid-crypto/src && echo "pub fn _stub() {}" > nyxid-crypto/src/lib.rs \
    && mkdir -p permissions/src && echo "fn main() {}" > permissions/src/main.rs \
    && echo "" > permissions/src/lib.rs \
    && mkdir -p machine/src && echo "" > machine/src/lib.rs \
    && mkdir -p machine-updater/src && echo "fn main() {}" > machine-updater/src/main.rs \
    && echo "" > machine-updater/src/lib.rs
RUN cargo build --locked --release --manifest-path backend/Cargo.toml --bin nyxid-server --features gcp-kms

FROM dependencies AS builder
# Restore every original manifest and the exact lock before compiling real code.
# CARGO_PKG_VERSION and workspace path dependency identities must be real here.
COPY --from=dependency-recipe /input/ ./

# Remove the dummy build artifacts (forces recompilation of our code only).
# Drop the cloud-auth artifacts too so its real source is recompiled.
RUN rm -rf backend/src cloud-auth/src service-adapters/src machine/src machine-updater/src permissions/src \
    target/release/deps/nyxid_machine_updater-* target/release/deps/libnyxid_machine_updater* \
    target/release/libnyxid_machine_updater.* \
    target/release/deps/nyxid_permissions-* \
    target/release/libnyxid_permissions.* \
    target/release/deps/nyxid_machine-* \
    target/release/libnyxid_machine.* \
    target/release/deps/nyxid_service_adapters-* \
    target/release/libnyxid_service_adapters.* \
    target/release/deps/nyxid-* \
    target/release/deps/nyxid_cloud_auth-* \
    target/release/libnyxid_cloud_auth.* \
    target/release/nyxid-server

# Copy the real source code, build script, docs, prompts, and catalog spec overlays
# (docs, prompts, specs, and assets are embedded via include_str!/include_bytes!)
COPY backend/src backend/src
COPY backend/build.rs backend/build.rs
COPY backend/prompts backend/prompts
COPY backend/content backend/content
COPY backend/specs backend/specs
COPY backend/assets backend/assets
COPY cloud-auth/src cloud-auth/src
COPY service-adapters/src service-adapters/src
COPY machine/src machine/src
COPY permissions/src permissions/src
COPY permissions/examples permissions/examples
COPY machine/resources machine/resources
COPY machine-updater/src machine-updater/src
COPY cli/src/tls.rs cli/src/tls.rs
COPY cli/src/tls cli/src/tls
COPY cli/src/commands/update_attestation.rs cli/src/commands/update_attestation.rs
COPY cli/src/commands/update_attestation cli/src/commands/update_attestation
COPY cli/resources/sigstore-root.json cli/resources/sigstore-root.json
COPY docs/AI_AGENT_PLAYBOOK.md docs/AI_AGENT_PLAYBOOK.md
COPY docs/assistant/CHANNEL_SETUP_GUIDE.md docs/assistant/CHANNEL_SETUP_GUIDE.md
# The oracle worker bundle is embedded via include_str! and served from
# /api/v1/oracle/worker-bundle so installs stay in sync with the server.
COPY integrations/oracle/cdp-worker/worker.mjs integrations/oracle/cdp-worker/worker.mjs

# Touch entry points so cargo sees them as newer than the cached dummies.
RUN touch backend/src/main.rs cloud-auth/src/lib.rs service-adapters/src/lib.rs machine/src/lib.rs machine-updater/src/lib.rs permissions/src/lib.rs

# Build the real binary without a per-commit input. The runtime stage stamps
# /health below; unchanged Rust/embedded inputs reuse this entire layer even
# when the release commit changes. .git is excluded from the build context.
RUN cargo build --locked --release --manifest-path backend/Cargo.toml --bin nyxid-server --features gcp-kms

# The standalone page is served by the backend with its own SRI-bound CSP.
# Keep the Node/lockfile/source/epoch inputs identical to the frontend build.
FROM node:22.23.2-alpine AS credential-builder
WORKDIR /build/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
ARG NYXID_GIT_HASH=unknown
ARG SOURCE_DATE_EPOCH
ENV SOURCE_COMMIT=${NYXID_GIT_HASH}
ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}
RUN npm run build:credential-accept

# ── Runtime ─────────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

# Create a non-root user
RUN groupadd -r nyxid && useradd -r -g nyxid -d /app -s /sbin/nologin nyxid

WORKDIR /app

# Copy the compiled binary
# The backend binary is `nyxid-server` (see backend/Cargo.toml [[bin]] block)
# to avoid colliding with the CLI's `nyxid` binary at the same workspace
# `target/{triple}/release/` path.
COPY --from=builder /build/target/release/nyxid-server /usr/local/bin/nyxid-server
COPY --from=credential-builder /build/frontend/dist/credential-accept /app/frontend/dist/credential-accept
COPY --from=credential-builder /build/frontend/dist/release-integrity /app/frontend/dist/release-integrity

# Create directory for JWT keys (mounted at runtime)
RUN mkdir -p /app/keys /var/lib/nyxid/tuf \
    && chown nyxid:nyxid /app/keys /var/lib/nyxid/tuf \
    && chmod 0700 /var/lib/nyxid/tuf

# Retain authenticated TUF rollback floors independently of the image/HOME.
ENV NYXID_SERVER_TUF_STATE_DIR=/var/lib/nyxid/tuf
VOLUME ["/var/lib/nyxid/tuf"]

USER nyxid

EXPOSE 3001

# Image identity must not invalidate either Cargo build. Non-container builds
# retain the build.rs fallback; the credential page keeps its own release stamp.
ARG NYXID_GIT_HASH=unknown
ENV NYXID_GIT_HASH=${NYXID_GIT_HASH}

ENTRYPOINT ["nyxid-server"]
