#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 3 ]]; then
  echo "Usage: $0 <service> <method> <path> [json-body]" >&2
  exit 1
fi

if [[ -z "${NYXID_BASE_URL:-}" ]]; then
  echo "NYXID_BASE_URL is required" >&2
  exit 1
fi

if [[ -z "${NYXID_ACCESS_TOKEN:-}" ]]; then
  echo "NYXID_ACCESS_TOKEN is required" >&2
  exit 1
fi

SERVICE="$1"
METHOD="$2"
PATH_PART="${3#/}"
BODY="${4:-}"
URL="${NYXID_BASE_URL%/}/api/v1/proxy/s/${SERVICE}/${PATH_PART}"

if [[ -n "$BODY" ]]; then
  curl -fsS -X "$METHOD" \
    -H "Authorization: Bearer ${NYXID_ACCESS_TOKEN}" \
    -H "Content-Type: application/json" \
    "$URL" \
    --data "$BODY"
else
  curl -fsS -X "$METHOD" \
    -H "Authorization: Bearer ${NYXID_ACCESS_TOKEN}" \
    "$URL"
fi
