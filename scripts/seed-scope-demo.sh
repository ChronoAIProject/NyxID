#!/usr/bin/env bash
# Seed a demo user + org + catalog-backed services so the RolePermissionCard /
# MemberScopeDialog grids on the org detail page have real rows to render.
#
# Requires: NyxID backend running on http://localhost:3001, MongoDB running.

set -euo pipefail

API="${NYXID_API:-http://localhost:3001/api/v1}"
EMAIL="${SEED_EMAIL:-admin@nyxid.dev}"
PASSWORD="${SEED_PASSWORD:-Password123!}"
ORG_NAME="${SEED_ORG:-Scope Icons Demo}"

json() { python3 -c "import json,sys; print(json.load(sys.stdin)$1)"; }

echo "→ register $EMAIL (idempotent — may 409)"
curl -sS -X POST "$API/auth/register" \
  -H 'Content-Type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\",\"display_name\":\"Scope Admin\"}" \
  -o /dev/null -w "  HTTP %{http_code}\n" || true

echo "→ login"
LOGIN_RESP=$(curl -sS -X POST "$API/auth/login" \
  -H 'Content-Type: application/json' \
  -H 'X-Client: cli' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}")
TOKEN=$(printf '%s' "$LOGIN_RESP" | json "['access_token']")
if [ -z "$TOKEN" ] || [ "$TOKEN" = "None" ]; then
  echo "  login failed: $LOGIN_RESP"
  exit 1
fi
AUTH=(-H "Authorization: Bearer $TOKEN")

echo "→ create org '$ORG_NAME' (idempotent — dedup by name)"
EXISTING=$(curl -sS "${AUTH[@]}" "$API/orgs" | python3 -c "
import json,sys
data = json.load(sys.stdin)
name = '$ORG_NAME'
for o in data.get('orgs', []):
    if o.get('display_name') == name:
        print(o['id'])
        break
")
if [ -n "$EXISTING" ]; then
  ORG_ID="$EXISTING"
  echo "  reusing existing org $ORG_ID"
else
  ORG_RESP=$(curl -sS -X POST "$API/orgs" "${AUTH[@]}" \
    -H 'Content-Type: application/json' \
    -d "{\"display_name\":\"$ORG_NAME\"}")
  ORG_ID=$(printf '%s' "$ORG_RESP" | json "['id']")
  echo "  created org $ORG_ID"
fi

echo "→ provision catalog services under org $ORG_ID"
# Slug → dummy credential. The credential value isn't exercised because we
# never actually proxy through these; the row just needs to exist so it
# shows up in the scope grid.
declare -a SERVICES=(
  "llm-openai:sk-demo-openai-key-000000000000000"
  "llm-anthropic:sk-ant-demo-key-000000000000000"
  "llm-google-ai:AIzaDemoGoogleKey000000000000000"
  "llm-deepseek:sk-demo-deepseek-key-00000000000"
  "llm-mistral:demo-mistral-key-0000000000000000"
  "llm-cohere:demo-cohere-key-000000000000000000"
  "api-github-pat:ghp_DemoDemoDemoDemoDemoDemoDemoDemoDe"
)

for entry in "${SERVICES[@]}"; do
  SLUG="${entry%%:*}"
  CRED="${entry#*:}"
  LABEL="$(printf '%s' "$SLUG" | tr '[:lower:]-' '[:upper:] ') demo"
  RESP=$(curl -sS -X POST "$API/keys" "${AUTH[@]}" \
    -H 'Content-Type: application/json' \
    -d "{\"service_slug\":\"$SLUG\",\"credential\":\"$CRED\",\"label\":\"$LABEL\",\"target_org_id\":\"$ORG_ID\"}")
  ID=$(printf '%s' "$RESP" | python3 -c "import json,sys;d=json.load(sys.stdin);print(d.get('id','') or d.get('error',''))")
  echo "  $SLUG → $ID"
done

echo
echo "──────────────────────────────────────────────────"
echo "Done. Open the frontend and log in:"
echo "  URL      http://localhost:3000/login"
echo "  Email    $EMAIL"
echo "  Password $PASSWORD"
echo
echo "Then browse to the org detail page and open the Role Permissions tab:"
echo "  http://localhost:3000/orgs/$ORG_ID"
echo "──────────────────────────────────────────────────"
