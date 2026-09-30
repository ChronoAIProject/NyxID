#!/usr/bin/env python3
"""Fingerprint the measurement recipe and this coverage job's runner image."""

import hashlib
import os
from pathlib import Path
import platform


INPUTS = (
    ".github/workflows/ci.yml",
    ".github/scripts/coverage_identity.py",
    ".github/scripts/ci_resources.py",
    ".github/scripts/start-test-mongodb.sh",
)


def fingerprint(root, image_os, image_version, architecture):
    digest = hashlib.sha256()
    for value in (image_os, image_version, architecture):
        digest.update(value.encode() + b"\0")
    for name in INPUTS:
        digest.update(name.encode() + b"\0")
        digest.update((root / name).read_bytes() + b"\0")
    return digest.hexdigest()


def main():
    image_os = os.environ.get("ImageOS", "")
    image_version = os.environ.get("ImageVersion", "")
    # Missing image identity makes cached report reuse unsafe; measure fresh.
    reusable = bool(image_os and image_version)
    value = fingerprint(Path.cwd(), image_os, image_version, platform.machine())
    print(f"Coverage recipe: {value}; runner image: {image_os}/{image_version}; reusable: {reusable}")
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        output.write(f"fingerprint={value}\nreusable={str(reusable).lower()}\n")


if __name__ == "__main__":
    main()
