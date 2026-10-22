#!/usr/bin/env python3
"""Find a published exact-SHA baseline, then validate its measurement identity.

This only transports existing reports; it is not a coverage measurement input.
Missing runs/artifacts or incompatible identities leave the PR delta unavailable.
"""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess

from coverage_summary import line_pct


BASELINE_ARTIFACT = "backend-coverage-baseline"
PUBLISH_WORKFLOWS = {".github/workflows/ci.yml", ".github/workflows/publish-images.yml"}


def find_baseline(runs, base_sha, artifacts_for_run):
    """Search newest successful trusted runs, skipping absent/expired artifacts."""
    for run in sorted(runs, key=lambda item: item["id"], reverse=True):
        if (run.get("head_sha") != base_sha or run.get("conclusion") != "success"
                or run.get("event") not in {"push", "workflow_dispatch"}
                or run.get("path") not in PUBLISH_WORKFLOWS):
            continue
        artifacts = [artifact for artifact in artifacts_for_run(run["id"])
                     if artifact.get("name") == BASELINE_ARTIFACT
                     and not artifact.get("expired", True)]
        if artifacts:
            artifact = max(artifacts, key=lambda item: item["id"])
            return {"run-id": run["id"], "artifact-id": artifact["id"]}
    return None


def validate_baseline(directory, fingerprint, reusable):
    if not reusable:
        return False
    try:
        identity = json.loads((directory / "coverage-backend.identity.json").read_text())
        if (not isinstance(identity, dict) or identity.get("fingerprint") != fingerprint
                or identity.get("reusable") is not True):
            return False
        percentage = line_pct(json.loads((directory / "coverage-backend.base.json").read_text()))
        return 0 <= percentage <= 100
    except (OSError, ValueError, TypeError, KeyError, AttributeError, SystemExit):
        return False


def api_items(endpoint, key):
    result = subprocess.run(["gh", "api", "--paginate", "--slurp", endpoint],
                            check=True, capture_output=True, text=True)
    return [item for page in json.loads(result.stdout) for item in page[key]]


def outputs(values):
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        for key, value in values.items():
            output.write(f"{key}={value}\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    select = commands.add_parser("select")
    select.add_argument("--repository", required=True)
    select.add_argument("--sha", required=True)
    verify = commands.add_parser("verify")
    verify.add_argument("--directory", type=Path, required=True)
    verify.add_argument("--fingerprint", required=True)
    verify.add_argument("--reusable", choices=("true", "false"), required=True)
    verify.add_argument("--output", type=Path, default=Path("coverage-backend.base.json"))
    args = parser.parse_args()

    if args.command == "select":
        try:
            runs = api_items(f"repos/{args.repository}/actions/runs?head_sha={args.sha}"
                             "&status=success&per_page=100", "workflow_runs")
            selected = find_baseline(runs, args.sha, lambda run_id: api_items(
                f"repos/{args.repository}/actions/runs/{run_id}/artifacts?per_page=100",
                "artifacts"))
        except (subprocess.CalledProcessError, OSError, ValueError, KeyError, TypeError):
            print("Base coverage delta: N/A (GitHub baseline lookup unavailable).")
            selected = None
        outputs(selected or {"run-id": "", "artifact-id": ""})
        print(f"Published baseline: {selected or 'unavailable for this exact base SHA'}")
    else:
        available = validate_baseline(args.directory, args.fingerprint, args.reusable == "true")
        if available:
            shutil.copyfile(args.directory / "coverage-backend.base.json", args.output)
        outputs({"available": str(available).lower()})
        print(f"Published baseline matches measurement identity: {available}")


if __name__ == "__main__":
    main()
