#!/usr/bin/env bash
# Seed the extra data needed to hand-test the four "A 1-4" icon surfaces:
#   A1  cli-wizard CatalogGrid          → /cli/pair  (kind=ai-key)
#   A2  SA "Connect Service" dropdown   → /admin/service-accounts/{sa}
#   A3  org approval add-policy select  → /orgs/{org}?tab=approvals
#   A4  cli-wizard AccessScopeCard      → /cli/pair  (kind=api-key-create)
#
# Depends on scripts/seed-scope-demo.sh having run first (org + services).
# Requires the backend on :3001. Pairing codes expire in 15 min — re-run
# this script (or just its pairing block) to mint fresh ones.

set -euo pipefail

API="${NYXID_API:-http://localhost:3001/api/v1}"
FE="${NYXID_FE:-http://localhost:3000}"
EMAIL="${SEED_EMAIL:-admin@nyxid.dev}"
PASSWORD="${SEED_PASSWORD:-Password123!}"
ORG_NAME="${SEED_ORG:-Scope Icons Demo}"

jqpy() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

TOKEN=$(curl -sS -X POST "$API/auth/login" \
  -H 'Content-Type: application/json' -H 'X-Client: cli' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}" | jqpy "d['access_token']")
AUTH=(-H "Authorization: Bearer $TOKEN")

ORG_ID=$(curl -sS "${AUTH[@]}" "$API/orgs" | python3 -c "
import json,sys
for o in json.load(sys.stdin).get('orgs', []):
    if o.get('display_name') == '$ORG_NAME':
        print(o['id']); break
")

# ── A2: an org-owned service account (dropdown needs an SA detail page) ─
# Scoped to the org so the demo user (org admin, not platform admin) can
# both create it and view it at /orgs/{org}/service-accounts/{sa}.
echo '→ org service account "Icon Demo SA" (idempotent by name)'
SA_ID=$(curl -sS "${AUTH[@]}" "$API/admin/service-accounts?org_id=$ORG_ID" | python3 -c "
import json,sys
d = json.load(sys.stdin)
rows = d.get('service_accounts', d if isinstance(d, list) else [])
for s in rows:
    if s.get('name') == 'Icon Demo SA':
        print(s['id']); break
" 2>/dev/null || true)
if [ -z "$SA_ID" ]; then
  SA_ID=$(curl -sS -X POST "$API/admin/service-accounts" "${AUTH[@]}" \
    -H 'Content-Type: application/json' \
    -d "{\"name\":\"Icon Demo SA\",\"description\":\"brand-icon dropdown demo\",\"allowed_scopes\":\"proxy\",\"target_org_id\":\"$ORG_ID\"}" \
    | jqpy "d.get('id','')")
fi
echo "  SA $SA_ID"

# ── A1 + A4: mint two CLI-pairing codes ──────────────────────────────
mint_pairing() {
  local kind="$1"
  curl -sS -X POST "$API/cli-pairings" "${AUTH[@]}" \
    -H 'Content-Type: application/json' \
    -d "{\"kind\":\"$kind\"}" | jqpy "d['code']"
}
echo '→ mint pairing codes (15-min TTL)'
AI_KEY_CODE=$(mint_pairing "ai-key")
API_KEY_CODE=$(mint_pairing "api-key-create")

cat <<EOF

════════════════════════════════════════════════════════════════════
  Log in first:  $FE/login    ($EMAIL / $PASSWORD)
  (this user is a platform admin — re-login / hard-refresh if the
   /admin nav isn't showing yet)
════════════════════════════════════════════════════════════════════

A3  Org approval — add-policy service select (brand icons in dropdown)
    $FE/orgs/$ORG_ID?tab=approvals
    → click "Add Policy" → open the Service dropdown.

A2  Service-account "Connect Service" dropdown (brand icons per row)
    $FE/admin/service-accounts/$SA_ID
    → click "Connect Service".  (admin route — the org SA page hides
      this section; log in as the promoted admin below)

A1  CLI-wizard CatalogGrid (brand icon on each service card)
    $FE/cli/pair
    → enter pairing code:  $AI_KEY_CODE

A4  CLI-wizard AccessScopeCard (brand icons in the Services checklist)
    $FE/cli/pair
    → enter pairing code:  $API_KEY_CODE

  (pairing codes expire in 15 min — re-run this script for fresh ones)
════════════════════════════════════════════════════════════════════
EOF
