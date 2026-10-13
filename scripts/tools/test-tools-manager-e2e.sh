#!/usr/bin/env bash
# Local-only proof; no production credentials or services.
set -euo pipefail
cd "$(dirname "$0")/../.."
case "${1:-tools}" in
  tools) test_filter=handlers::tools_manager_tests::tools_manager_local_fake_vendor_e2e ;;
  catalog) test_filter=handlers::tools_manager_tests::catalog_e2e::catalog_manager_local_fake_oauth_e2e ;;
  *) echo 'Usage: test-tools-manager-e2e.sh [tools|catalog]' >&2; exit 2 ;;
esac
command -v mongod >/dev/null
command -v mongosh >/dev/null
# Transaction fixtures create many WiredTiger files. Use a bounded cache and
# raise the local file limit when the host permits it.
if [[ $(ulimit -n) -lt 65536 ]]; then ulimit -n 65536; fi
run_dir=$(mktemp -d "${TMPDIR:-/tmp}/nyxid-tools-e2e.XXXXXX")
port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
mongo_pid=""
cleanup() {
  local exit_status=$?
  if [[ -n "$mongo_pid" ]]; then kill "$mongo_pid" 2>/dev/null || true; wait "$mongo_pid" 2>/dev/null || true; fi
  if [[ "$exit_status" -ne 0 ]]; then
    # Retain bounded, redacted diagnostics before removing only this run's DB.
    python3 - "$run_dir" <<'PY'
from pathlib import Path
import re, sys
run = Path(sys.argv[1])
output = run.parent / (run.name + '.diagnostics.log')
parts = []
for name in ('launch.log', 'init.log', 'readiness.log', 'mongo.log'):
    path = run / name
    if path.exists():
        lines = path.read_text(errors='replace').splitlines()[-40:]
        text = '\n'.join(lines)[-8192:]
        text = re.sub(r'(?i)(authorization|password|client_secret|access_token|refresh_token|credential)([\s"=:]+)[^\s,}]+', r'\1\2[REDACTED]', text)
        parts.append(f'{name}:\n{text}')
output.write_text('\n\n'.join(parts)[-24576:] + '\n')
print(f'Owned-Mongo failure diagnostics: {output}', file=sys.stderr)
PY
  fi
  rm -rf "$run_dir"
}
trap cleanup EXIT
mkdir "$run_dir/db"
mongod --dbpath "$run_dir/db" --bind_ip 127.0.0.1 --port "$port" --replSet tools_e2e --wiredTigerCacheSizeGB 0.25 --setParameter diagnosticDataCollectionEnabled=false --logpath "$run_dir/mongo.log" > "$run_dir/launch.log" 2>&1 &
mongo_pid=$!
mongo_uri="mongodb://127.0.0.1:$port/?directConnection=true&serverSelectionTimeoutMS=1000&connectTimeoutMS=1000"
ready=false
for attempt in $(seq 1 50); do
  kill -0 "$mongo_pid" 2>/dev/null || break
  if mongosh --quiet "$mongo_uri" --eval 'db.adminCommand({ping:1})' > "$run_dir/readiness.log" 2>&1; then ready=true; break; fi
  sleep 0.2
done
if [[ "$ready" != true ]]; then echo 'Owned Mongo did not become reachable' >&2; exit 1; fi
mongosh --quiet "$mongo_uri" --eval "rs.initiate({_id:'tools_e2e',members:[{_id:0,host:'127.0.0.1:$port'}]})" > "$run_dir/init.log" 2>&1
ready=false
for attempt in $(seq 1 50); do
  kill -0 "$mongo_pid" 2>/dev/null || break
  if mongosh --quiet "$mongo_uri" --eval 'const hello=db.hello(); quit(hello.isWritablePrimary === true && hello.setName === "tools_e2e" ? 0 : 1)' > "$run_dir/readiness.log" 2>&1; then ready=true; break; fi
  sleep 0.2
done
if [[ "$ready" != true ]]; then echo 'Owned replica set did not elect its primary' >&2; exit 1; fi
export NYXID_TEST_DATABASE_URL="mongodb://127.0.0.1:$port/?replicaSet=tools_e2e&directConnection=true"
export CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2
export NYXID_NO_UPDATE_CHECK=1 NYXID_SKIP_SKILL_SELF_HEAL=1 NYXID_SHARE_ANALYTICS=0 DO_NOT_TRACK=1
cargo build -p nyxid-cli --bin nyxid --locked
export NYXID_TOOLS_E2E_CLI="$(pwd)/target/debug/nyxid"
if [[ -n "${NYXID_TOOLS_E2E_TEST_BIN:-}" ]]; then
  # Reuse a test executable compiled from the recorded source snapshot. The
  # caller records its hash; no repeated backend build is needed for filters.
  case "$NYXID_TOOLS_E2E_TEST_BIN" in
    "$(pwd)/target/debug/deps/nyxid_server-"*) ;;
    *) echo 'E2E test binary must belong to this checkout' >&2; exit 1 ;;
  esac
  [[ -x "$NYXID_TOOLS_E2E_TEST_BIN" ]]
  "$NYXID_TOOLS_E2E_TEST_BIN" "$test_filter" --exact --ignored --nocapture --test-threads=1
else
  cargo test -p nyxid --bin nyxid-server "$test_filter" --locked -- --exact --ignored --nocapture --test-threads=1
fi
