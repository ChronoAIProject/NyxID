# ── Stage 1: Build ────────────────────────────────────────────────────────────
FROM rust:1.93-bookworm AS builder

WORKDIR /build

# Copy workspace manifests and lock file for dependency caching
COPY Cargo.toml Cargo.lock ./
COPY backend/Cargo.toml backend/Cargo.toml
COPY node-agent/Cargo.toml node-agent/Cargo.toml

# Create dummy main.rs files so cargo can resolve and compile dependencies
RUN mkdir -p backend/src && echo "fn main() {}" > backend/src/main.rs \
    && mkdir -p node-agent/src && echo "fn main() {}" > node-agent/src/main.rs
RUN cargo build --release --manifest-path backend/Cargo.toml --features gcp-kms

# Remove the dummy build artifacts (forces recompilation of our code only)
RUN rm -rf backend/src target/release/deps/nyxid-* target/release/nyxid

# Copy the real source code
COPY backend/src backend/src

# Touch main.rs to ensure cargo sees it as newer than the cached dummy
RUN touch backend/src/main.rs

# Build the real binary
RUN cargo build --release --manifest-path backend/Cargo.toml --features gcp-kms

# ── Stage 2: Runtime ─────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl openssh-client \
    && rm -rf /var/lib/apt/lists/*

# Create a non-root user
RUN groupadd -r nyxid && useradd -r -g nyxid -d /app -s /sbin/nologin nyxid

WORKDIR /app

# Copy the compiled binary
COPY --from=builder /build/target/release/nyxid /usr/local/bin/nyxid

# Create directory for JWT keys (mounted at runtime)
RUN mkdir -p /app/keys && chown nyxid:nyxid /app/keys

USER nyxid

EXPOSE 3001

ENTRYPOINT ["nyxid"]
