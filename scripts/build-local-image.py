#!/usr/bin/env python3
"""Build a local image from an exact clean committed source export.

Dirty checkouts fail before Docker runs. Native Cargo builds remain available
for investigation and retain their git-unverified/git-dirty identity.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


source = module("image_source", "export-build-source.py")
runner = module("image_runner", "run-exported-build.py")


def build(root, destination, dockerfile, tag, target=None, platform=None, contexts=()):
    commit = source.git(root, "rev-parse", "HEAD").decode().strip()
    relative = str(source.relative(dockerfile))
    exported = source.export(root, destination, commit)
    tree = destination / "source"
    file = tree / relative
    if file.is_symlink() or not file.is_file():
        raise ValueError("Dockerfile must be a regular committed source input")
    command = ["docker", "build", "--build-arg", "NYXID_GIT_HASH=" + commit,
               "--build-arg", "SOURCE_DATE_EPOCH=" + str(exported["source"]["epoch"]),
               "--file", str(file), "--tag", tag]
    if target:
        command += ["--target", target]
    if platform:
        command += ["--platform", platform]
    for context in contexts:
        command += ["--build-context", context]
    command.append(str(tree))
    code = runner.run(destination, commit, destination / "image-build-execution.json", command)
    return code, exported


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--file", required=True, help="committed Dockerfile relative to repository root")
    parser.add_argument("--tag", required=True)
    parser.add_argument("--target")
    parser.add_argument("--platform")
    parser.add_argument("--build-context", action="append", default=[],
                        help="additional input; its identity requires separate verification")
    parser.add_argument("--destination", type=Path, help="new path outside Git for retained inputs/receipts")
    args = parser.parse_args()
    destination = args.destination or Path(tempfile.mkdtemp(prefix="nyxid-local-image-")) / "export"
    code, receipt = build(args.root.resolve(), destination.resolve(), args.file,
                          args.tag, args.target, args.platform, args.build_context)
    print(json.dumps({"source": receipt["source"], "archive": receipt["archive"],
                      "evidence": str(destination), "exit_code": code}))
    raise SystemExit(code)


if __name__ == "__main__":
    main()
