#!/usr/bin/env bash
# Failure/cancellation diagnostics for the per-job test MongoDB container.
# Metadata and server log tail only; test databases contain fixtures, never
# production data, and nothing here prints environment variables.
set -uo pipefail

name=nyxid-test-mongodb
if ! docker inspect "$name" >/dev/null 2>&1; then
  echo "test MongoDB container not found"
  exit 0
fi
docker inspect "$name" --format \
  'status={{.State.Status}} exit_code={{.State.ExitCode}} oom_killed={{.State.OOMKilled}} error={{.State.Error}} started={{.State.StartedAt}} finished={{.State.FinishedAt}}'
pid="$(docker inspect "$name" --format '{{.State.Pid}}')"
if [ -n "$pid" ] && [ "$pid" != "0" ] && [ -d "/proc/$pid" ]; then
  echo "mongod open descriptors: $(sudo ls "/proc/$pid/fd" 2>/dev/null | wc -l)"
  sudo grep -E 'Max open files|Max processes' "/proc/$pid/limits" 2>/dev/null || true
  sudo grep -E 'VmRSS|VmHWM' "/proc/$pid/status" 2>/dev/null || true
fi
echo "--- kernel OOM messages"
sudo dmesg 2>/dev/null | grep -iE 'out of memory|oom-kill|killed process' | tail -n 20 || true
echo "--- mongod log tail"
docker logs --tail 200 "$name" 2>&1 | tail -n 200
