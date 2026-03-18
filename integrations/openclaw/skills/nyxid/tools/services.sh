#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${NYXID_BASE_URL:-}" ]]; then
  echo "NYXID_BASE_URL is required" >&2
  exit 1
fi

if [[ -z "${NYXID_ACCESS_TOKEN:-}" ]]; then
  echo "NYXID_ACCESS_TOKEN is required" >&2
  exit 1
fi

curl -fsS \
  -H "Authorization: Bearer ${NYXID_ACCESS_TOKEN}" \
  "${NYXID_BASE_URL%/}/api/v1/proxy/services"
