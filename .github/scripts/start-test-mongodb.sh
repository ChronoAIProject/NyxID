#!/usr/bin/env bash
set -euo pipefail

# This container is dedicated to one CI job. Disable MongoDB 8's default
# 300-second snapshot window so dropped per-test idents do not exhaust FDs.
# Single-process coverage keeps thousands of per-test databases (collections
# plus indexes, each a WiredTiger file) open at once, so the descriptor limit
# must be far above the nextest peak. The test data set is tiny: cap the
# WiredTiger cache instead of letting it claim half of the runner's RAM next
# to the instrumented test binary.
docker run --detach --name nyxid-test-mongodb --publish 27017:27017 \
  --ulimit nofile=1048576:1048576 \
  mongo:8.0@sha256:4968f22d0c6c10ef29952f3e807f62872ba22b3312f25803564fbfc08255efc2 --replSet rs0 --bind_ip_all \
  --wiredTigerCacheSizeGB 2 \
  --setParameter minSnapshotHistoryWindowInSeconds=0
for attempt in {1..30}; do
  if docker exec nyxid-test-mongodb mongosh --quiet \
    --eval 'db.adminCommand({ ping: 1 }).ok' >/dev/null 2>&1; then
    break
  fi
  if [ "$attempt" -eq 30 ]; then
    docker logs nyxid-test-mongodb
    exit 1
  fi
  sleep 1
done
snapshot_window="$(docker exec nyxid-test-mongodb mongosh --quiet --eval \
  'db.adminCommand({getParameter:1,minSnapshotHistoryWindowInSeconds:1}).minSnapshotHistoryWindowInSeconds')"
if [ "$snapshot_window" != "0" ]; then
  echo "expected minSnapshotHistoryWindowInSeconds=0, got: $snapshot_window"
  exit 1
fi
docker exec nyxid-test-mongodb mongosh --quiet --eval \
  'rs.initiate({_id:"rs0",members:[{_id:0,host:"127.0.0.1:27017"}]})'
for attempt in {1..30}; do
  if docker exec nyxid-test-mongodb mongosh --quiet \
    --eval 'db.hello().isWritablePrimary' | grep -q true; then
    break
  fi
  if [ "$attempt" -eq 30 ]; then
    docker logs nyxid-test-mongodb
    exit 1
  fi
  sleep 1
done
