#!/usr/bin/env python3
"""Materialize complete expected subject coverage before building/publishing."""
import argparse
import json
from pathlib import Path
import subprocess

from publication_contract import ROOT, dist_contract, image_contract, load_json, validate_plan


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("producer", choices=("cli", "images"))
    parser.add_argument("--dist-plan", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    epoch = int(subprocess.check_output(["git", "show", "-s", "--format=%ct", "HEAD"], cwd=ROOT, text=True).strip())
    if args.producer == "cli":
        if not args.dist_plan:
            parser.error("CLI requires the actual validated --dist-plan")
        metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--no-deps", "--format-version=1"], cwd=ROOT, text=True))
        plan = dist_contract(load_json(args.dist_plan), metadata, commit, epoch)
    else:
        from workflow_yaml import load_unique
        workflow = load_unique((ROOT / ".github/workflows/publish-images.yml").read_text())
        plan = image_contract(commit, epoch, workflow["jobs"]["build"]["strategy"]["matrix"])
    validate_plan(plan)
    args.output.write_text(json.dumps(plan, indent=2) + "\n")
    print("Complete " + args.producer + " publication plan: " + str(len(plan["subjects"])) + " subjects")
