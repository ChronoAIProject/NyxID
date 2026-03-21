#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${NYXID_BASE_URL:-https://nyx-api.chrono-ai.fun}"

auth_args=()
if [[ -n "${NYXID_API_KEY:-}" ]]; then
  auth_args=(-H "X-API-Key: ${NYXID_API_KEY}")
elif [[ -n "${NYXID_ACCESS_TOKEN:-}" ]]; then
  auth_args=(-H "Authorization: Bearer ${NYXID_ACCESS_TOKEN}")
else
  echo "Set NYXID_API_KEY or NYXID_ACCESS_TOKEN before calling NyxID." >&2
  exit 1
fi

curl -fsS "${auth_args[@]}" "${BASE_URL%/}/api/v1/proxy/services"
