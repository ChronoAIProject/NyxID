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

# Compatibility wrapper. The supported planner owns every read and write.
python3 - "$work_dir/manifest.json" "$source_slug" "$tool_slug" "$supplier" "$credential_env" "${#topics[@]}" ${topics[@]+"${topics[@]}"} "${operations[@]}" <<'PYCODE'
import json,sys
count=int(sys.argv[6]); tool={"twin_of":sys.argv[2],"slug":sys.argv[3],"name":sys.argv[4],"supplier":sys.argv[4],"topics":sys.argv[7:7+count],"operations":sys.argv[7+count:]}
if sys.argv[5]: tool["credential_env"]=sys.argv[5]
with open(sys.argv[1],"w") as f: json.dump({"version":1,"tools":[tool]},f)
PYCODE
"$cli" admin tools apply --file "$work_dir/manifest.json"
"$cli" admin tools validate --file "$work_dir/manifest.json"
