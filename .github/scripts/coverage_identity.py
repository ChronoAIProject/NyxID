#!/usr/bin/env python3
"""Fingerprint the measurement recipe and require matching runner OS identities."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform


INPUTS = (
    ".github/workflows/ci.yml",
    ".github/workflows/backend-tests.yml",
    ".config/nextest.toml",
    ".github/scripts/backend_shards.py",
    ".github/scripts/coverage_identity.py",
    ".github/scripts/ci_resources.py",
    ".github/scripts/start-test-mongodb.sh",
    ".github/scripts/prepare-backend-runner.sh",
    ".github/scripts/install-linux-build-deps.sh",
)


def fingerprint(root, image_os, architecture):
    digest = hashlib.sha256()
    for value in (image_os, architecture):
        digest.update(value.encode() + b"\0")
    for name in INPUTS:
        digest.update(name.encode() + b"\0")
        digest.update((root / name).read_bytes() + b"\0")
    return digest.hexdigest()


def matching_identities(paths, value):
    for path in paths:
        try:
            identity = json.loads(path.read_text())
        except (OSError, ValueError):
            return False
        if not isinstance(identity, dict) or identity.get("fingerprint") != value \
                or identity.get("reusable") is not True:
            return False
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", type=Path)
    parser.add_argument("--require-identities", type=Path, nargs="*", default=[])
    args = parser.parse_args()
    image_os = os.environ.get("ImageOS", "")
    value = fingerprint(Path.cwd(), image_os, platform.machine())
    # Tools are pinned in the recipe. Weekly image builds may differ, while
    # runner OS, architecture, and recipe must agree across all measurements.
    reusable = bool(image_os) and matching_identities(args.require_identities, value)
    if args.record:
        args.record.write_text(json.dumps({"fingerprint": value, "reusable": reusable}))
    print(f"Coverage recipe: {value}; runner OS: {image_os}; reusable: {reusable}")
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        output.write(f"fingerprint={value}\nreusable={str(reusable).lower()}\n")


if __name__ == "__main__":
    main()
