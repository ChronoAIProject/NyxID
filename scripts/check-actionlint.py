#!/usr/bin/env python3
"""Validate GitHub workflow semantics with the checksum-pinned release binary.

No PATH-installed actionlint or fetched installer is trusted. Shellcheck and
Pyflakes are separate optional tools, so disable their discovery for identical
GitHub syntax/expression/call-graph validation on developer and hosted machines.
No actionlint diagnostic is ignored.
"""
import argparse
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def install(bin_dir):
    subprocess.run([sys.executable, str(ROOT / "scripts/bootstrap-build-tool.py"),
                    "actionlint", "--bin-dir", str(bin_dir)], check=True)
    return bin_dir.resolve() / "actionlint"


def validate(binary, root):
    paths = sorted(path for path in (root / ".github/workflows").iterdir()
                   if path.suffix in (".yml", ".yaml"))
    if not paths:
        raise ValueError("no workflows to validate")
    return subprocess.run([str(binary), "-oneline", "-shellcheck=", "-pyflakes=",
                           *[str(path.relative_to(root)) for path in paths]],
                          cwd=root, text=True, capture_output=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    result = validate(install(args.bin_dir), ROOT)
    sys.stdout.write(result.stdout)
    sys.stderr.write(result.stderr)
    if result.returncode == 0:
        print("Pinned actionlint: every workflow passed GitHub semantic validation")
    return result.returncode


if __name__ == "__main__":
    sys.exit(main())
