#!/usr/bin/env python3
"""Plan/remove superseded CI caches, never a release-dist cache or live lane.

Input: gh cache list --limit 10000 --json id,key,ref,sizeInBytes,createdAt,lastAccessedAt
Default is a dry run. --apply is used only by the trusted main-branch workflow.
"""
import argparse
import json
import re
import subprocess
from pathlib import Path

# rust-cache appends OS, architecture, environment hash, and dependency hash.
RUST = re.compile(r"^v0-rust-(.+)-Linux-x64-[0-9a-f]{8}-[0-9a-f]{8}$")
LANES = {
    "permissions-test", "rust-clippy", "backend-build", "coverage-backend-build",
    "backend-image-inputs-backend-image-inputs", "cli-cli-test",
    "features-aws-kms-rust-features", "features-gcp-kms-rust-features",
    "features-aws-and-gcp-kms-rust-features", "coverage-cli-coverage-cli",
}
RETIRED = {
    "wizard-freshness-wizard-bundle-freshness": "cli-cli-test",
    "backend-billing-smoke-backend-billing-smoke": "backend-build",
}
MAIN = "refs/heads/main"


def plan(rows):
    newest = {}
    for row in rows:
        match = RUST.fullmatch(row["key"])
        if match and match[1] in LANES and row["ref"] == MAIN:
            lane = match[1]
            if lane not in newest or (row["createdAt"], row["id"]) > (
                newest[lane]["createdAt"], newest[lane]["id"]
            ):
                newest[lane] = row
    removals = []
    for row in rows:
        key = row["key"]
        # Defense in depth: this invariant applies even to unexpected platforms.
        if key.startswith("v0-rust-dist-"):
            continue
        reason = None
        if key.startswith(("buildkit-blob-", "index-machine-pr-", "index-backend-pr-")):
            reason = "Docker consumers now import registry caches; GHA export retired"
        elif (match := RUST.fullmatch(key)):
            lane = match[1]
            replacement = newest.get(RETIRED.get(lane, lane))
            if replacement and row["id"] != replacement["id"]:
                reason = f"superseded by main cache {replacement['id']}"
        if reason:
            removals.append({**row, "reason": reason})
    return removals


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("inventory", type=Path)
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--repo", required=True)
    args = parser.parse_args()
    rows = json.loads(args.inventory.read_text())
    removals = plan(rows)
    ids = {row["id"] for row in removals}
    kept = [row for row in rows if row["id"] not in ids]
    total = sum(row["sizeInBytes"] for row in rows)
    remaining = sum(row["sizeInBytes"] for row in kept)
    ordinary = sum(row["sizeInBytes"] for row in kept if not row["key"].startswith("v0-rust-dist-"))
    print(f"Before: {len(rows)} entries, {total / 1e9:.3f} GB")
    print(f"Planned: remove {len(removals)}, retain {remaining / 1e9:.3f} GB")
    print(f"Non-dist retained: {ordinary / 1e9:.3f} GB; dist capacity: {(10 * 1024**3 - ordinary) / 1e9:.3f} GB")
    if ordinary > 10 * 1024**3 - 4_000_000_000:
        print("::warning::Less than 4 GB reserved for dist. Review sizes; live CI caches were not evicted.")
    for row in removals:
        print(f"{row['id']} {row['sizeInBytes'] / 1e6:.1f} MB {row['key']}: {row['reason']}")
        if args.apply:
            subprocess.run(["gh", "cache", "delete", str(row["id"]), "--repo", args.repo], check=True)


if __name__ == "__main__":
    main()
