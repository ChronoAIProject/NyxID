#!/usr/bin/env bash
# Local-only proof; no production credentials or services.
set -euo pipefail
cd "$(dirname "$0")/../.."
command -v mongod >/dev/null
command -v mongosh >/dev/null
# Transaction fixtures create many WiredTiger files. Use a bounded cache and
# raise the local file limit when the host permits it.
if [[ $(ulimit -n) -lt 65536 ]]; then ulimit -n 65536; fi
run_dir=$(mktemp -d "${TMPDIR:-/tmp}/nyxid-tools-e2e.XXXXXX")
port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
mongo_pid=""
cleanup() {
  if [[ -n "$mongo_pid" ]]; then kill "$mongo_pid" 2>/dev/null || true; wait "$mongo_pid" 2>/dev/null || true; fi
  rm -rf "$run_dir"
}
trap cleanup EXIT
mkdir "$run_dir/db"
mongod --dbpath "$run_dir/db" --bind_ip 127.0.0.1 --port "$port" --replSet tools_e2e --wiredTigerCacheSizeGB 0.25 --setParameter diagnosticDataCollectionEnabled=false --logpath "$run_dir/mongo.log" > "$run_dir/launch.log" 2>&1 &
mongo_pid=$!
for attempt in $(seq 1 50); do
  if mongosh --quiet --host 127.0.0.1 --port "$port" --eval 'db.adminCommand({ping:1})' >/dev/null 2>&1; then break; fi
  sleep 0.2
done
mongosh --quiet --host 127.0.0.1 --port "$port" --eval "rs.initiate({_id:'tools_e2e',members:[{_id:0,host:'127.0.0.1:$port'}]})" > "$run_dir/init.log"
export NYXID_TEST_DATABASE_URL="mongodb://127.0.0.1:$port/?replicaSet=tools_e2e&directConnection=true"
export CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2
export NYXID_NO_UPDATE_CHECK=1 NYXID_SKIP_SKILL_SELF_HEAL=1 NYXID_SHARE_ANALYTICS=0 DO_NOT_TRACK=1
cargo build -p nyxid-cli --bin nyxid --locked
export NYXID_TOOLS_E2E_CLI="$(pwd)/target/debug/nyxid"
cargo test -p nyxid --bin nyxid-server tools_manager_local_fake_vendor_e2e --locked -- --ignored --nocapture
