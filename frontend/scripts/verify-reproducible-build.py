#!/usr/bin/env python3
"""Build the actual SPA/credential assets twice and compare every shipped byte.

This checks frontend bytes for the recorded toolchain/inputs, not OS image or
Rust binary reproducibility. Evidence and the first build stay outside source.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess


def inventory(directory):
    result = {}
    for path in sorted(directory.rglob("*")):
        if path.is_symlink():
            raise ValueError("unexpected symlink in frontend output: " + str(path))
        if path.is_file():
            result[str(path.relative_to(directory))] = hashlib.sha256(path.read_bytes()).hexdigest()
    if not result:
        raise ValueError("empty frontend output")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence-dir", type=Path, required=True)
    parser.add_argument("--npm", default="npm", help="npm executable; install locked dependencies before running")
    args = parser.parse_args()
    frontend = Path(__file__).resolve().parents[1]
    root = frontend.parent
    output = args.evidence_dir.resolve()
    output.mkdir(parents=True, exist_ok=False)
    git = lambda *argv: subprocess.check_output(["git", *argv], cwd=root).strip()
    commit = git("rev-parse", "HEAD").decode()
    epoch = git("show", "-s", "--format=%ct", "HEAD").decode()
    receipt = {"source_commit": commit, "source_date_epoch": epoch,
               "tracked_diff_sha256": hashlib.sha256(git("diff", "--binary", "HEAD")).hexdigest(),
               "node": subprocess.check_output(["node", "--version"], text=True).strip(),
               "npm": subprocess.check_output([args.npm, "--version"], text=True).strip(),
               "builds": []}
    inventories = []
    for index, threads in enumerate(("1", "8"), 1):
        env = {**os.environ, "SOURCE_COMMIT": commit, "SOURCE_DATE_EPOCH": epoch,
               "UV_THREADPOOL_SIZE": threads}
        with (output / f"build-{index}.log").open("w") as log:
            subprocess.run([args.npm, "run", "build"], cwd=frontend, env=env,
                           stdout=log, stderr=subprocess.STDOUT, check=True)
        files = inventory(frontend / "dist")
        inventories.append(files)
        (output / f"build-{index}-sha256.json").write_text(json.dumps(files, sort_keys=True, indent=2) + "\n")
        receipt["builds"].append({"uv_threadpool_size": threads, "files": len(files)})
        if index == 1:
            shutil.copytree(frontend / "dist", output / "first-dist")
    before, after = inventories
    receipt.update({"matches": before == after,
                    "missing": sorted(before.keys() - after.keys()),
                    "extra": sorted(after.keys() - before.keys()),
                    "different": sorted(name for name in before.keys() & after.keys() if before[name] != after[name])})
    (output / "comparison.json").write_text(json.dumps(receipt, indent=2) + "\n")
    if not receipt["matches"]:
        raise SystemExit("frontend build differs; inspect comparison.json and first-dist")
    print(f"Both builds match: {len(before)} files, thread pools1/8; evidence {output}")


if __name__ == "__main__":
    main()
