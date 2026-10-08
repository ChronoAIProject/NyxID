#!/usr/bin/env bash
set -euo pipefail
umask 077

usage() {
  printf '%s\n' 'Usage: add-tool-twin.sh --source <slug> --slug <tools-slug> --supplier <name> [--topic <slug>]... --publish <operation>... [--credential-env <variable>]'
}
source_slug='' tool_slug='' supplier='' credential_env=''
topics=() operations=()
while (($#)); do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --source|--slug|--supplier|--topic|--publish|--credential-env)
      (($# >= 2)) || { usage >&2; exit 2; }
      case "$1" in
        --source) source_slug=$2 ;;
        --slug) tool_slug=$2 ;;
        --supplier) supplier=$2 ;;
        --topic) topics+=("$2") ;;
        --publish) operations+=("$2") ;;
        --credential-env) credential_env=$2 ;;
      esac
      shift 2 ;;
    *) usage >&2; exit 2 ;;
  esac
done
[[ -n "$source_slug" && -n "$tool_slug" && -n "$supplier" && ${#operations[@]} -gt 0 ]] || { usage >&2; exit 2; }
cli=${NYXID_BIN:-nyxid}
work_dir=$(mktemp -d)
trap 'rm -rf "$work_dir"' EXIT

# All server reads and writes go through the CLI; Python only compares JSON.
"$cli" catalog list --all --output json > "$work_dir/catalog.json"
exists=$(python3 - "$work_dir/catalog.json" "$tool_slug" <<'PY'
import json,sys
value=json.load(open(sys.argv[1]));rows=value.get('entries',[]) if isinstance(value,dict) else value
print('yes' if any(row.get('slug')==sys.argv[2] for row in rows) else 'no')
PY
)
if [[ "$exists" == no ]]; then
  args=(service add --catalog-admin --twin-of "$source_slug" --slug "$tool_slug" --offering-kind tool --supplier "$supplier")
  for topic in ${topics[@]+"${topics[@]}"}; do args+=(--topic "$topic"); done
  "$cli" "${args[@]}" --output json > "$work_dir/created.json"
  printf 'Created %s from %s\n' "$tool_slug" "$source_slug"
fi
"$cli" service show "$tool_slug" --catalog-admin --output json > "$work_dir/service.json"
metadata=$(python3 - "$work_dir/service.json" "$source_slug" "$supplier" ${topics[@]+"${topics[@]}"} <<'PY'
import json,sys
row=json.load(open(sys.argv[1]));source=row.get('import_source') or {}
if row.get('offering_kind')!='tool' or source.get('kind')!='catalog_twin' or source.get('reference')!=sys.argv[2]:
    raise SystemExit('Existing slug is not a twin of the requested source; refusing to change it')
print('same' if row.get('supplier')==sys.argv[3] and sorted(row.get('topics',[]))==sorted(sys.argv[4:]) else 'changed')
PY
)
if [[ "$metadata" == changed ]]; then
  args=(service update --catalog-admin "$tool_slug" --supplier "$supplier")
  if ((${#topics[@]})); then
    for topic in ${topics[@]+"${topics[@]}"}; do args+=(--topic "$topic"); done
  else
    args+=(--clear-topics)
  fi
  "$cli" "${args[@]}" --output json > "$work_dir/updated.json"
  printf 'Updated metadata for %s\n' "$tool_slug"
fi
credential_changed=no
if [[ -n "$credential_env" ]]; then
  configured=$(python3 - "$work_dir/service.json" <<'PY'
import json,sys
print('yes' if json.load(open(sys.argv[1])).get('credential_configured') else 'no')
PY
)
  if [[ "$configured" == no ]]; then
    "$cli" service update --catalog-admin "$tool_slug" --credential-env "$credential_env" --output json > "$work_dir/credential.json"
    credential_changed=yes
    printf 'Configured credential for %s\n' "$tool_slug"
  fi
fi
"$cli" catalog endpoint list "$tool_slug" --output json > "$work_dir/endpoints.json"
python3 - "$work_dir/endpoints.json" "$work_dir" "${operations[@]}" <<'PY'
import json,sys
from pathlib import Path
rows=json.load(open(sys.argv[1]))['endpoints'];wanted=set(sys.argv[3:]);known={row['name'] for row in rows}
if wanted-known: raise SystemExit('Unknown operations: '+', '.join(sorted(wanted-known)))
for name in wanted:
    if '\n' in name or '\r' in name: raise SystemExit('Invalid operation name')
for action in ['publish','pause']:
    selected=([row['name'] for row in rows if row['name'] in wanted and (row.get('publication')!='published' or not row.get('is_active'))] if action=='publish' else [row['name'] for row in rows if row['name'] not in wanted and row.get('publication')=='published'])
    Path(sys.argv[2],action).write_text(''.join(name+'\n' for name in selected))
PY
changed=no
for action in pause publish; do
  args=(catalog publish "$tool_slug")
  count=0
  while IFS= read -r operation; do args+=(--operation "$operation"); count=$((count+1)); done < "$work_dir/$action"
  if ((count)); then
    state=published
    [[ "$action" == pause ]] && state=paused
    "$cli" "${args[@]}" --state "$state" --output json > "$work_dir/$action.json"
    printf '%s: %s operations -> %s\n' "$tool_slug" "$count" "$state"
    changed=yes
  fi
done
if [[ "$exists" == yes && "$metadata" == same && "$changed" == no && "$credential_changed" == no ]]; then
  printf '%s unchanged (no-op)\n' "$tool_slug"
fi
