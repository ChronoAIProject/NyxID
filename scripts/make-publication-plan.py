#!/usr/bin/env python3
"""Materialize complete expected subject coverage before building/publishing."""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess

from publication_contract import ROOT, dist_contract, image_contract, load_json, validate_plan


spec = importlib.util.spec_from_file_location('publication_source_export', Path(__file__).with_name('export-build-source.py'))
exporter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(exporter)


def checkout_identity(root):
    # Reuse isolated, exact-path ownership trust for GitHub container jobs.
    # Never alter global safe.directory or trust an unrelated checkout.
    commit = exporter.head_commit(root)
    epoch = int(exporter.git(root, 'show', '-s', '--format=%ct', commit).decode().strip())
    return commit, epoch


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("producer", choices=("cli", "images"))
    parser.add_argument("--dist-plan", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    commit, epoch = checkout_identity(ROOT)
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
