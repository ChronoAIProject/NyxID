FROM rust:1.98.1-slim-bookworm AS builder
RUN rustup component add clippy
WORKDIR /build
COPY cli/tests/machine_context_spike/Cargo.toml cli/tests/machine_context_spike/Cargo.lock cli/tests/machine_context_spike/
COPY cli/tests/machine_context_spike/src/ cli/tests/machine_context_spike/src/
COPY cli/src/node/machine/process.rs cli/src/node/machine/context_sandbox.rs cli/src/node/machine/
RUN cargo +1.98.1 clippy --locked --manifest-path cli/tests/machine_context_spike/Cargo.toml --all-targets -- -D warnings \
    && cargo +1.98.1 test --locked --manifest-path cli/tests/machine_context_spike/Cargo.toml \
    && RUST_MIN_STACK=1572864 cargo +1.98.1 test --locked --manifest-path cli/tests/machine_context_spike/Cargo.toml \
    && cargo +1.98.1 build --locked --manifest-path cli/tests/machine_context_spike/Cargo.toml

FROM scratch AS artifact
COPY --from=builder /build/cli/tests/machine_context_spike/target/debug/nyxid-machine-context-spike /nyxid-context-spike

FROM ghcr.io/chronoaiproject/nyxid/nyxid-node-machine@sha256:2f1fe82573fa86f99c3cca219be33ab846f17a7374cc40bcf51e2a56a1b4f7d4 AS spike
RUN apt-get update && apt-get install -y --no-install-recommends cargo python3-pip python3-venv \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/cli/tests/machine_context_spike/target/debug/nyxid-machine-context-spike /usr/local/bin/nyxid-context-spike
COPY cli/tests/machine_context_spike/check.py /opt/m13/check.py
ENTRYPOINT ["python3", "/opt/m13/check.py"]
