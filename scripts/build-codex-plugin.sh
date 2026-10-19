#!/usr/bin/env bash
# Build the OpenAI plugin directory ZIP (ChatGPT and Codex) from
# integrations/codex-plugin/. The archive contains exactly the package folder's
# allowlisted contents; nothing outside the folder is copied in.
#
#   scripts/build-codex-plugin.sh            validate and write dist/nyxid-codex-plugin-<version>.zip
#   scripts/build-codex-plugin.sh --check    validate only (used by CI)
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
package="$repo_root/integrations/codex-plugin"
allowlist=(plugin.json mcp.json README.md LICENSE assets skills)

fail() { printf 'Codex plugin validation failed: %s\n' "$1" >&2; exit 1; }

check_only=false
[[ "${1:-}" == "--check" ]] && check_only=true

jq -e . "$package/plugin.json" "$package/mcp.json" >/dev/null || fail "plugin.json or mcp.json is not valid JSON"
version="$(jq -r '.version // empty' "$package/plugin.json")"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "plugin.json version must be semantic versioning"
[[ "$(jq -r '.name' "$package/plugin.json")" == nyxid ]] || fail "plugin.json name must be nyxid"
jq -e '.mcpServers | length > 0 and all(.[]; (.url | startswith("https://")) and (has("headers") | not))' \
  "$package/mcp.json" >/dev/null || fail "mcp.json must declare https servers without embedded headers"
jq -e '.extensions["com.openai"].interface | (.defaultPrompt | length >= 3) and (.privacyPolicyURL | startswith("https://")) and (.termsOfServiceURL | startswith("https://"))' \
  "$package/plugin.json" >/dev/null || fail "interface needs at least 3 default prompts and https privacy/terms URLs"
for icon in $(jq -r '.extensions["com.openai"].interface | .composerIcon, .logo' "$package/plugin.json"); do
  [[ -f "$package/$icon" ]] || fail "icon does not exist: $icon"
done

# Every top-level entry must be allowlisted, and every allowlisted entry present.
while IFS= read -r entry; do
  name="$(basename "$entry")"
  [[ " ${allowlist[*]} " == *" $name "* ]] || fail "unexpected entry in package: $name"
done < <(find "$package" -mindepth 1 -maxdepth 1)
for name in "${allowlist[@]}"; do
  [[ -e "$package/$name" ]] || fail "missing required entry: $name"
done
[[ -z "$(find "$package" -type l)" ]] || fail "symlinks are not allowed in the package"
[[ -z "$(find "$package" -name .DS_Store)" ]] || fail ".DS_Store files are not allowed in the package"

skills=("$package"/skills/*/SKILL.md)
[[ -f "${skills[0]}" ]] || fail "package must bundle at least one skills/<name>/SKILL.md"
for skill in "${skills[@]}"; do
  head -1 "$skill" | grep -qx -- '---' || fail "$skill: missing YAML frontmatter"
  grep -q '^name:' "$skill" && grep -q '^description:' "$skill" || fail "$skill: frontmatter requires name and description"
done

if $check_only; then
  printf 'Codex plugin validation passed (version %s)\n' "$version"
  exit 0
fi

archive="$repo_root/dist/nyxid-codex-plugin-$version.zip"
mkdir -p "$repo_root/dist"
rm -f "$archive"
(cd "$package" && zip -X -q -r "$archive" "${allowlist[@]}")
unzip -tqq "$archive"
printf '%s\n' "$archive"
