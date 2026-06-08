# ── Stage 1: Build ────────────────────────────────────────────────────────────
FROM rust:1.93-bookworm AS builder

WORKDIR /build

# Commit hash baked into the binary for /health. `.git` is excluded from the
# Docker context, so CI must pass this ARG explicitly; otherwise the build
# script falls back to "unknown".
ARG NYXID_GIT_HASH=unknown
ENV NYXID_GIT_HASH=${NYXID_GIT_HASH}

# Copy workspace manifests and lock file for dependency caching.
# `cloud-auth/` (NyxID#716) and `nyxid-crypto/` (NyxID#773) are workspace
# members; cargo refuses to resolve the workspace if any member's manifest
# is missing, so they must be staged here even though the backend binary
# only links cloud-auth directly via path (nyxid-crypto is CLI-only).
COPY Cargo.toml Cargo.lock ./
COPY backend/Cargo.toml backend/Cargo.toml
COPY cli/Cargo.toml cli/Cargo.toml
COPY cloud-auth/Cargo.toml cloud-auth/Cargo.toml
COPY mcp-demo/Cargo.toml mcp-demo/Cargo.toml
COPY nyxid-crypto/Cargo.toml nyxid-crypto/Cargo.toml

# Create dummy entry points so cargo can resolve and compile dependencies.
# backend, cli, and mcp-demo are binary crates (need main.rs).
# cloud-auth and nyxid-crypto are library crates (need lib.rs); a stub
# `pub fn _stub(){}` is enough to satisfy cargo's check that the lib target
# exists. nyxid-crypto isn't in the backend binary's graph, so its stub is
# never compiled -- it only needs to exist for workspace resolution.
RUN mkdir -p backend/src && echo "fn main() {}" > backend/src/main.rs \
    && mkdir -p cli/src && echo "fn main() {}" > cli/src/main.rs \
    && mkdir -p cloud-auth/src && echo "pub fn _stub() {}" > cloud-auth/src/lib.rs \
    && mkdir -p mcp-demo/src && echo "fn main() {}" > mcp-demo/src/main.rs \
    && mkdir -p nyxid-crypto/src && echo "pub fn _stub() {}" > nyxid-crypto/src/lib.rs
RUN cargo build --release --manifest-path backend/Cargo.toml --features gcp-kms

# Remove the dummy build artifacts (forces recompilation of our code only).
# Drop the cloud-auth artifacts too so its real source is recompiled.
RUN rm -rf backend/src cloud-auth/src \
    target/release/deps/nyxid-* \
    target/release/deps/nyxid_cloud_auth-* \
    target/release/libnyxid_cloud_auth.* \
    target/release/nyxid-server

# Copy the real source code, build script, and docs (docs are embedded via include_str!)
COPY backend/src backend/src
COPY backend/build.rs backend/build.rs
COPY cloud-auth/src cloud-auth/src
COPY docs/AI_AGENT_PLAYBOOK.md docs/AI_AGENT_PLAYBOOK.md

# Touch entry points so cargo sees them as newer than the cached dummies.
RUN touch backend/src/main.rs cloud-auth/src/lib.rs

# Build the real binary
RUN cargo build --release --manifest-path backend/Cargo.toml --features gcp-kms

# ── Stage 2: Runtime ─────────────────────────────────────────────────────────
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

# Create directory for JWT keys (mounted at runtime)
RUN mkdir -p /app/keys && chown nyxid:nyxid /app/keys

USER nyxid

EXPOSE 3001

ENTRYPOINT ["nyxid-server"]
