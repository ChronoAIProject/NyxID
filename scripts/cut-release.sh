#!/usr/bin/env bash
set -euo pipefail

die() { printf '%s\n' "$*" >&2; exit 1; }
usage="Usage: scripts/cut-release.sh [--commit <sha>] [--supersede] [--dry-run]"
commit=
supersede=false
dry_run=false
while (($#)); do
  case "$1" in
    --commit)
      [[ ${2:-} =~ ^[0-9a-fA-F]{4,40}$ ]] || die "$usage"
      commit=$2
      shift 2 ;;
    --supersede) supersede=true; shift ;;
    --dry-run) dry_run=true; shift ;;
    --help|-h) printf '%s\n' "$usage"; exit 0 ;;
    *) die "$usage" ;;
  esac
done

# Bind every API request to origin, including in a checkout with multiple remotes.
origin=$(git remote get-url origin)
case "$origin" in
  git@github.com:*|https://github.com/*|ssh://git@github.com/*)
    repo=${origin#*github.com?}
    repo=${repo%/}
    repo=${repo%.git} ;;
  *) die "Expected a github.com origin remote." ;;
esac
[[ $repo =~ ^[[:alnum:]_.-]+/[[:alnum:]_.-]+$ ]] || die "Invalid GitHub repository in origin."
export GH_HOST=github.com
api="repos/$repo"

staging_commit() {
  curl --fail --silent --show-error --max-time 20 -H 'Cache-Control: no-cache' "$1" |
    python3 -c '
import json, re, sys
sha = json.load(sys.stdin).get("commit")
if not isinstance(sha, str) or not re.fullmatch(r"[0-9a-fA-F]{12}", sha):
    sys.exit("Expected a 12-hex staging commit")
print(sha.lower())'
}

if [[ -z $commit ]]; then
  backend=$(staging_commit https://staging-api.nyxid.ai/health) || die "Could not read the staging backend commit."
  frontend=$(staging_commit https://staging.nyxid.ai/build-version.json) || die "Could not read the staging frontend commit."
  [[ $backend == "$frontend" ]] || die "Staging commits differ. Backend $backend, frontend $frontend."
  commit=$backend
fi

sha=$(gh api "$api/commits/$commit" --jq '.sha')
shopt -s nocasematch
[[ $sha =~ ^[0-9a-f]{40}$ && $sha == "$commit"* ]] || die "Could not resolve $commit to a matching commit SHA."
shopt -u nocasematch
status=$(gh api "$api/compare/dev...$sha?per_page=1" --jq '.status')
case "$status" in
  behind|identical) ;;
  *) die "$sha is outside origin/dev history (comparison: $status)." ;;
esac

# Pin both ends across pages. Follow only parent zero through the commits
# GitHub lists as reachable from the snapshot and missing from main.
main_sha=$(gh api "$api/git/ref/heads/main" --jq '.object.sha')
changes=$(gh api --paginate --slurp "$api/compare/$main_sha...$sha?per_page=100" | python3 -c '
import json, re, subprocess, sys
pages = json.load(sys.stdin)
snapshot, main, api = sys.argv[1:]
status = pages[0]["status"]
if status in ("identical", "behind"):
    sys.exit(f"{snapshot} is already contained in main.")
commits = {c["sha"]: c for page in pages for c in page["commits"]}
if status not in ("ahead", "diverged") or snapshot not in commits or len(commits) != pages[0]["total_commits"]:
    sys.exit("Incomplete comparison with main.")

def api_pages(path):
    return json.loads(subprocess.check_output(["gh", "api", "--paginate", "--slurp", f"{api}/{path}"]))

def associated_prs(sha):
    return [pr for page in api_pages(f"commits/{sha}/pulls?per_page=100") for pr in page
            if pr["merged_at"] and pr["merge_commit_sha"] == sha]

def is_rollup_merge(commit, missing):
    parents = commit["parents"]
    if len(parents) != 2 or parents[1]["sha"] in missing:
        return False
    for pr in associated_prs(commit["sha"]):
        base, head = pr["base"], pr["head"]
        if base["ref"] != "main" or not head["repo"] or head["repo"]["id"] != base["repo"]["id"]:
            continue
        if head["ref"] == "dev" or head["ref"].startswith("release/"):
            return True
    return False

if status == "diverged":
    # Ordinary rollups leave merge commits on main. Every other main-only
    # commit (including a hotfix merge) must reach the selected snapshot.
    reverse = api_pages(f"compare/{snapshot}...{main}?per_page=100")
    missing = {c["sha"]: c for page in reverse for c in page["commits"]}
    if main not in missing or len(missing) != reverse[0]["total_commits"]:
        sys.exit("Incomplete comparison with the release snapshot.")
    if not all(is_rollup_merge(commit, missing) for commit in missing.values()):
        sys.exit("Merge main into dev first, then cut from a snapshot containing the sync.")

entries = []
current = snapshot
while current in commits:
    commit = commits[current]
    lines = commit["commit"]["message"].splitlines()
    title = lines[0] if lines else "(empty commit message)"
    merge = re.match(r"Merge pull request #(\d+) from ", title)
    if merge and len(lines) > 2 and lines[2]:
        title = f"{lines[2]} (#{merge[1]})"
    elif not re.search(r"\(#\d+\)$", title):
        # A maintainer can edit the merge message and omit the PR number.
        for pr in associated_prs(current):
            if pr["base"]["ref"] == "dev":
                title = "{} (#{})".format(pr["title"], pr["number"])
    entries.append(f"- {title} ({current[:7]})")
    parents = commit["parents"]
    current = parents[0]["sha"] if parents else None
print("\n".join(reversed(entries)))' "$sha" "$main_sha" "$api")

day=$(python3 -c 'from datetime import datetime, timezone; print(datetime.now(timezone.utc).date())')
branch="release/$day"
branches=$(gh api --paginate "$api/git/matching-refs/heads/$branch" --jq '.[].ref')
suffix=2
while [[ $'\n'$branches$'\n' == *$'\n'"refs/heads/$branch"$'\n'* ]]; do
  branch="release/$day-$suffix"
  suffix=$((suffix + 1))
done
open_prs=$(gh api --paginate "$api/pulls?state=open&base=main&per_page=100" --jq '
  .[] | select(.head.repo.id == .base.repo.id)
      | select(.head.ref == "dev" or (.head.ref | startswith("release/"))) | .html_url')
if [[ -n $open_prs && $supersede == false ]]; then
  die "Release PR already open (use --supersede to replace):
$open_prs"
fi

title="release: $day (${sha:0:7})"
body="Release commit $sha from dev.

First-parent commits since main

$changes"
printf 'Create %s at %s in %s\nOpen %s into main\n%s\n\n%s\n' "$branch" "$sha" "$repo" "$branch" "$title" "$body"
if $dry_run; then
  if [[ -n $open_prs ]]; then
    printf 'After opening the new PR, close with a comment linking to it:\n%s\n' "$open_prs"
  fi
  exit 0
fi

gh api --method POST "$api/git/refs" -f "ref=refs/heads/$branch" -f "sha=$sha" --silent
url=$(gh pr create --repo "$repo" --base main --head "$branch" --title "$title" --body-file - <<< "$body") ||
  die "Created $branch at $sha. Check its PR before retrying."
printf '%s\n' "$url"
while IFS= read -r old; do
  [[ -z $old ]] || gh pr close "$old" --comment "Superseded by $url."
done <<< "$open_prs"
