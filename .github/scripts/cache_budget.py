#!/usr/bin/env python3
"""Plan/remove cache entries whose restore paths are retired.

Input: gh cache list --limit 10000 --json id,key,ref,sizeInBytes,createdAt,lastAccessedAt
Default is a dry run. --apply is used only by the trusted main-branch workflow.
"""
import argparse
from collections import defaultdict
from datetime import datetime, timedelta, timezone
import json
import os
import re
import subprocess
from pathlib import Path

MAIN = "refs/heads/main"
RUST = re.compile(
    r"^v0-rust-(?P<lane>.+)-(?P<os>[^-]+)-(?P<arch>[^-]+)-[0-9a-f]{8}-[0-9a-f]{8}$"
)
INCREMENTAL = re.compile(
    r"^v0-rust-incremental-(?P<lane>.+)-(?P<os>[^-]+)-(?P<arch>[^-]+)-"
    r"[0-9a-f]+-[0-9a-f]+$"
)
DIST = re.compile(
    r"^v0-rust-dist-(?P<target>.+)-(?P<os>[^-]+)-(?P<arch>[^-]+)-[0-9a-f]{8}-[0-9a-f]{8}$"
)
NODE = re.compile(
    r"^node-cache-(?P<os>[^-]+)-(?P<arch>[^-]+)-(?P<manager>[^-]+)-[0-9a-f]+$"
)
COVERAGE_BASE = re.compile(
    r"^coverage-backend-base-v2-(?P<sha>[0-9a-f]{40})-(?P<fingerprint>[0-9a-f]{64})$"
)
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
DOCKER = ("buildkit-blob-", "index-machine-pr-", "index-backend-pr-")


def timestamp(value):
    """Return an aware UTC timestamp, or None for malformed inventory data."""
    if not isinstance(value, str):
        return None
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
        if parsed.tzinfo is None:
            parsed = parsed.replace(tzinfo=timezone.utc)
        return parsed.astimezone(timezone.utc)
    except ValueError:
        return None


def rank(row):
    parsed = timestamp(row.get("createdAt"))
    return (parsed or datetime.min.replace(tzinfo=timezone.utc), str(row.get("id", "")))


def family(key):
    if DIST.fullmatch(key):
        return "dist"
    if NODE.fullmatch(key):
        return "node"
    if COVERAGE_BASE.fullmatch(key):
        return "coverage-backend-base"
    if INCREMENTAL.fullmatch(key):
        return "rust-incremental"
    match = RUST.fullmatch(key)
    if match and (match["lane"] in LANES or match["lane"] in RETIRED):
        return "rust-lane"
    if key.startswith(DOCKER):
        return "docker"
    return "unknown"


def main_newest(rows, pattern, group):
    newest = {}
    for row in rows:
        if row["ref"] != MAIN:
            continue
        match = pattern.fullmatch(row["key"])
        if not match:
            continue
        key = group(match)
        previous = newest.get(key)
        if previous is None or rank(row) > rank(previous):
            newest[key] = row
    return newest


def plan(rows, now=None):
    """Return cache rows safe to delete, preserving every current restore lane."""
    now = now or datetime.now(timezone.utc)
    cutoff = now - timedelta(days=14)
    removals = []

    dist_newest = main_newest(rows, DIST, lambda match: match["target"])
    main_node_keys = {
        row["key"] for row in rows if row["ref"] == MAIN and NODE.fullmatch(row["key"])
    }
    coverage_rows = [
        row for row in rows if row["ref"] == MAIN and COVERAGE_BASE.fullmatch(row["key"])
    ]
    coverage_keep = {row["id"] for row in sorted(coverage_rows, key=rank, reverse=True)[:30]}
    coverage_keep.update(
        row["id"] for row in coverage_rows
        if (created := timestamp(row.get("createdAt"))) is not None and created >= cutoff
    )
    # A malformed timestamp cannot establish that a baseline is old. Preserve
    # it conservatively unless a later valid generation replaces it by count.
    coverage_keep.update(
        row["id"] for row in coverage_rows if timestamp(row.get("createdAt")) is None
    )

    rust_newest = {}
    for row in rows:
        if row["ref"] != MAIN:
            continue
        match = RUST.fullmatch(row["key"])
        if not match or match["lane"] not in LANES:
            continue
        family_key = (match["lane"], match["os"], match["arch"])
        if family_key not in rust_newest or rank(row) > rank(rust_newest[family_key]):
            rust_newest[family_key] = row

    incremental_newest = {}
    for row in rows:
        if row["ref"] != MAIN:
            continue
        match = INCREMENTAL.fullmatch(row["key"])
        if not match:
            continue
        family_key = (match["lane"], match["os"], match["arch"])
        if family_key not in incremental_newest or rank(row) > rank(incremental_newest[family_key]):
            incremental_newest[family_key] = row

    for row in rows:
        key = row["key"]
        match = DIST.fullmatch(key)
        reason = None
        if match:
            newest = dist_newest.get(match["target"])
            if row["ref"] != MAIN:
                reason = "dist cache saved on a non-main ref"
            elif newest is not None and row["id"] != newest["id"]:
                reason = f"superseded dist cache for target by main cache {newest['id']}"
        else:
            match = NODE.fullmatch(key)
            if match:
                if row["ref"] != MAIN and key in main_node_keys:
                    reason = "non-main node cache has an identical main cache"
            elif COVERAGE_BASE.fullmatch(key):
                if row["ref"] != MAIN:
                    reason = "coverage baseline saved on a non-main ref"
                elif row["id"] not in coverage_keep:
                    reason = "coverage baseline is older than the newest 30 and 14-day window"
            else:
                rust = RUST.fullmatch(key)
                if rust and rust["lane"] in (LANES | set(RETIRED)):
                    family_key = (
                        RETIRED.get(rust["lane"], rust["lane"]), rust["os"], rust["arch"]
                    )
                    replacement = rust_newest.get(family_key)
                    if replacement and row["id"] != replacement["id"]:
                        reason = f"superseded by main cache {replacement['id']}"
                elif key.startswith(DOCKER):
                    reason = "Docker consumers now import registry caches; GHA export retired"
                else:
                    incremental = INCREMENTAL.fullmatch(key)
                    if incremental:
                        family_key = (incremental["lane"], incremental["os"], incremental["arch"])
                        replacement = incremental_newest.get(family_key)
                        if replacement and row["id"] != replacement["id"]:
                            reason = f"superseded incremental cache by main cache {replacement['id']}"
        if reason:
            removals.append({**row, "reason": reason})
    return removals


def report(rows, removals, soft_cap_gb):
    remove_ids = {row["id"] for row in removals}
    totals = defaultdict(int)
    retained = defaultdict(int)
    for row in rows:
        totals[family(row["key"])] += row["sizeInBytes"]
        if row["id"] not in remove_ids:
            retained[family(row["key"])] += row["sizeInBytes"]
    total = sum(totals.values())
    remaining = sum(retained.values())
    print(f"Before: {len(rows)} entries, {total / 1e9:.3f} GB")
    print(f"Planned: remove {len(removals)}, retain {remaining / 1e9:.3f} GB")
    print("Family totals (before -> retained):")
    for name in sorted(totals):
        print(f"  {name}: {totals[name] / 1e9:.3f} -> {retained[name] / 1e9:.3f} GB")
    if remaining > soft_cap_gb * 1e9:
        print(
            f"::warning::Retained Actions cache total {remaining / 1e9:.3f} GB "
            f"exceeds the soft cap of {soft_cap_gb:.3f} GB; no newest entries were removed."
        )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("inventory", type=Path)
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--repo", required=True)
    parser.add_argument(
        "--soft-cap-gb", type=float,
        default=float(os.environ.get("CACHE_SOFT_CAP_GB", "40")),
    )
    args = parser.parse_args()
    if args.soft_cap_gb <= 0:
        parser.error("--soft-cap-gb must be positive")
    rows = json.loads(args.inventory.read_text())
    removals = plan(rows)
    report(rows, removals, args.soft_cap_gb)
    for row in removals:
        print(
            f"{row['id']} {row['sizeInBytes'] / 1e6:.1f} MB {row['ref']} "
            f"{row['key']}: {row['reason']}"
        )
        if args.apply:
            subprocess.run(["gh", "cache", "delete", str(row["id"]), "--repo", args.repo], check=True)


if __name__ == "__main__":
    main()
