#!/usr/bin/env bash
set -euo pipefail

# This container is dedicated to one CI job. Disable MongoDB 8's default
# 300-second snapshot window so dropped per-test idents do not exhaust FDs.
docker run --detach --name nyxid-test-mongodb --publish 27017:27017 \
  --ulimit nofile=65536:65536 \
  mongo:8.0 --replSet rs0 --bind_ip_all \
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
