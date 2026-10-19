#!/bin/bash
set -euo pipefail
for attempt in $(seq 1 60); do
  if mongosh --quiet --host mongodb --eval 'const admin = db.getSiblingDB("admin"); quit(admin.auth(process.env.MONGO_INITDB_ROOT_USERNAME, process.env.MONGO_INITDB_ROOT_PASSWORD) && (() => { const replication = admin.runCommand({getCmdLineOpts:1}).parsed.replication; return (replication?.replSet ?? replication?.replSetName) === "nyxid-rs"; })() ? 0 : 1)' >/dev/null 2>&1; then
    exec mongosh --quiet --host mongodb /scripts/init-replica.js
  fi
  sleep 1
done
echo "MongoDB replica initialization timed out: authenticated MongoDB with replica set nyxid-rs did not become available." >&2
exit 1
