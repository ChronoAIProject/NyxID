#!/bin/sh
set -eu
python3 /etc/nginx/nyxid/render-nginx.py
# No success marker is persisted. nginx returns 503 if this private checker
# stops or stalls, while its own /frontend/health continues to prove liveness.
python3 /etc/nginx/nyxid/frontend-readiness.py &
