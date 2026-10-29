#!/bin/sh
# Disposable actual CLI integration; no Chromium, Pod/CSI or billing evidence.
set -eu
repo=$(CDPATH= cd -- "$(dirname "$0")/../../.." && pwd)
runtime_test_target=${1:-/tmp/nyxid-hosted-linux-target}
if [ "$#" -gt 0 ]; then shift; fi
registry=${CARGO_HOME:-$HOME/.cargo}/registry
mkdir -p "$runtime_test_target"
runtime_test_target=$(CDPATH= cd -- "$runtime_test_target" && pwd)
docker build -f "$repo/cli/tests/managed_runtime_integration/Dockerfile" -t nyxid-managed-runtime-integration:local "$repo"
run_fixture() {
docker run --rm -i --init --read-only --cap-drop ALL \
    --cap-add CHOWN --cap-add SETUID --cap-add SETGID --cap-add KILL \
    --security-opt no-new-privileges \
    --mount "type=bind,source=$repo,target=/build,readonly" \
    --mount "type=bind,source=$registry,target=/cargo/registry,readonly" \
    --mount "type=bind,source=$runtime_test_target,target=/target" \
    --tmpfs /cargo:rw,nosuid,nodev,mode=0755 \
    --tmpfs /tmp:rw,nosuid,nodev \
    --tmpfs /workspace:rw,nosuid,nodev,mode=0755 \
    --volume /home \
    --tmpfs /var/lib/nyxid-contexts:rw,nosuid,nodev,mode=0711 \
    --tmpfs /var/lib/nyxid-hosted-tests:rw,nosuid,nodev,mode=0755 \
    -e CARGO_INCREMENTAL=1 -e CARGO_PROFILE_TEST_DEBUG=0 -e CARGO_BUILD_JOBS=1 -e CARGO_HOME=/cargo -e CARGO_TARGET_DIR=/target \
    -w /build nyxid-managed-runtime-integration:local python3 - "$@" <<'CHECKS'
import os,subprocess,sys
# This kernel's tmpfs lacks POSIX ACLs. Use a disposable anonymous disk volume
# for homes (removed by --rm), as the hosted disk emptyDir must support the real
# browser UID ACL. Do not bypass ACL checks or add capabilities for this test.
for uid,path in [(1000,'/home/agent'),(1001,'/home/browser'),(1002,'/home/devbrowser')]:
    os.mkdir(path,0o700)
    os.chown(path,uid,uid)
subprocess.run(['env','CARGO_INCREMENTAL=1',*sys.argv[1:]],check=True)
CHECKS
}
if [ "$#" -gt 0 ]; then
    run_fixture "$@"
    exit
fi
# Each suite receives its own original retained volume. No suite may inherit
# another test Node's journal; caches remain shared and are never cleaned.
run_fixture cargo test --offline --locked -p nyxid-cli --lib --features node-proxy-test context_dispatch::tests -- --include-ignored --test-threads=1 --nocapture
run_fixture cargo test --offline --locked -p nyxid-cli --lib --features node-proxy-test legacy_pinning_tests -- --include-ignored --test-threads=1 --nocapture
run_fixture cargo test --offline --locked -p nyxid-cli --lib --features node-proxy-test managed_configuration -- --include-ignored --test-threads=1 --nocapture
run_fixture cargo test --offline --locked -p nyxid-cli --bin nyxid managed_socket_ -- --include-ignored --test-threads=1 --nocapture
run_fixture cargo test --offline --locked -p nyxid-cli --bin nyxid node::managed_runtime::tests
run_fixture cargo test --offline --locked -p nyxid-cli --bin nyxid node::machine::browser::tests -- --include-ignored --test-threads=1 --nocapture
run_fixture cargo test --offline --locked -p nyxid-cli --bin nyxid browser_directory::tests -- --include-ignored --test-threads=1 --nocapture
run_fixture cargo test --offline --locked -p nyxid-cli --bin nyxid legacy_display_cookie_cleanup_ -- --include-ignored --test-threads=1 --nocapture
run_fixture cargo test --offline --locked -p nyxid-cli --bin nyxid browser_initialization_errors_are_consistent_for_browser_and_computer -- --test-threads=1 --nocapture
run_fixture cargo check --offline --locked -p nyxid-cli --lib --features node-proxy-test
run_fixture cargo check --offline --locked -p nyxid-cli --bin nyxid
