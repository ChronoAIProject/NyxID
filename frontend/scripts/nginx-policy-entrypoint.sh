#!/bin/sh
set -eu
if [ "${1:-}" = "--supervise-readiness" ]; then
    child_pid=
    stop_checker() {
        trap - TERM INT QUIT
        if [ -n "$child_pid" ]; then
            kill -TERM "$child_pid" 2>/dev/null || true
            wait "$child_pid" 2>/dev/null || true
        fi
        exit 0
    }
    trap stop_checker TERM INT QUIT
    backoff=1
    while :; do
        started=$(date +%s)
        python3 /etc/nginx/nyxid/frontend-readiness.py &
        child_pid=$!
        wait "$child_pid" || true
        child_pid=
        # A single crash retries in one second. Repeated immediate failures
        # cannot spin; a stable checker resets the capped restart backoff.
        if [ "$(( $(date +%s) - started ))" -ge 30 ]; then
            backoff=1
        fi
        sleep "$backoff" &
        child_pid=$!
        wait "$child_pid" || true
        child_pid=
        if [ "$backoff" -lt 8 ]; then
            backoff=$((backoff * 2))
        fi
    done
fi

python3 /etc/nginx/nyxid/render-nginx.py
# No success marker is persisted. nginx returns 503 if this private checker
# stops or stalls, while its own /frontend/health continues to prove liveness.
# This child supervisor survives the entrypoint hook and reaps every checker.
"$0" --supervise-readiness &
