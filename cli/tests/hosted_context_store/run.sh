#!/bin/sh
# Disposable Linux filesystem/sandbox checks. This is not a formal hosted Pod.
set -eu
repo=$(CDPATH= cd -- "$(dirname "$0")/../../.." && pwd)
runtime_test_target=${1:-/tmp/nyxid-hosted-linux-target}
registry=${CARGO_HOME:-$HOME/.cargo}/registry
mkdir -p "$runtime_test_target"
runtime_test_target=$(CDPATH= cd -- "$runtime_test_target" && pwd)
docker build -f "$repo/cli/tests/hosted_context_store/Dockerfile" -t nyxid-hosted-substrate-test:local "$repo"
docker run --rm --init --read-only --cap-drop ALL \
    --cap-add CHOWN --cap-add SETUID --cap-add SETGID --cap-add KILL \
    --security-opt no-new-privileges \
    --mount "type=bind,source=$repo,target=/build,readonly" \
    --mount "type=bind,source=$registry,target=/cargo/registry,readonly" \
    --mount "type=bind,source=$runtime_test_target,target=/target" \
    --tmpfs /cargo:rw,nosuid,nodev,mode=0755 \
    --tmpfs /tmp:rw,nosuid,nodev \
    --tmpfs /var/lib/nyxid-hosted-tests:rw,nosuid,nodev,mode=0755 \
    -e CARGO_INCREMENTAL=1 -e CARGO_HOME=/cargo -e CARGO_TARGET_DIR=/target \
    -w /build nyxid-hosted-substrate-test:local \
    cargo test --offline --manifest-path cli/tests/hosted_context_store/Cargo.toml -- --test-threads=1
